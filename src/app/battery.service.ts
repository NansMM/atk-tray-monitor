import { Injectable, inject } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as isAutostartEnabled,
} from '@tauri-apps/plugin-autostart';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { Store } from '@tauri-apps/plugin-store';
import { ReplaySubject, Subject } from 'rxjs';
import type { BatteryHistoryEntry, BatterySnapshot } from './battery.models';
import { I18nService } from './i18n.service';

const SETTINGS_FILE = 'settings.json';
const START_MINIMIZED_KEY = 'startMinimizedOnAutostart';
const LOW_BATTERY_NOTIFICATIONS_KEY = 'lowBatteryNotifications';
const LOW_BATTERY_THRESHOLD_KEY = 'lowBatteryThreshold';
const BATTERY_HISTORY_KEY = 'batteryHistory';
const SETTINGS_UPDATED_EVENT = 'settings-updated';

@Injectable({ providedIn: 'root' })
export class BatteryService {
  private readonly i18n = inject(I18nService);
  private readonly updatesSubject = new ReplaySubject<BatterySnapshot>(1);
  private readonly settingsUpdatesSubject = new Subject<void>();
  private readonly previewSnapshot = this.createPreviewSnapshot();
  private unlisten: UnlistenFn | null = null;
  private settingsStorePromise: Promise<Store> | null = null;

  readonly batteryUpdates$ = this.updatesSubject.asObservable();
  readonly settingsUpdates$ = this.settingsUpdatesSubject.asObservable();

  constructor() {
    this.updatesSubject.next(this.previewSnapshot);
    void this.bindBatteryEvents();
    void this.bindSettingsEvents();
  }

  async refreshBattery(): Promise<BatterySnapshot> {
    try {
      const snapshot = this.normalize(
        await invoke<BatterySnapshot>('refresh_battery_status'),
      );

      this.updatesSubject.next(snapshot);
      return snapshot;
    } catch {
      const preview = this.createPreviewSnapshot();
      this.updatesSubject.next(preview);
      return preview;
    }
  }

  async hideWindow(): Promise<void> {
    try {
      await invoke('hide_window');
    } catch {
      return;
    }
  }

  async showWindow(): Promise<void> {
    try {
      await invoke('show_window');
    } catch {
      return;
    }
  }

  async fitWindowToContent(contentHeight: number): Promise<void> {
    const normalizedHeight = Math.max(1, Math.ceil(contentHeight));

    try {
      await invoke('fit_window_to_content', {
        contentHeight: normalizedHeight,
      });
    } catch {
      return;
    }
  }

  async getLaunchOnStartup(): Promise<boolean> {
    try {
      return await isAutostartEnabled();
    } catch {
      return false;
    }
  }

  async setLaunchOnStartup(enabled: boolean): Promise<boolean> {
    try {
      if (enabled) {
        await enableAutostart();
      } else {
        await disableAutostart();
      }

      return await isAutostartEnabled();
    } catch {
      return false;
    }
  }

  async getStartMinimizedOnAutostart(): Promise<boolean> {
    return (await this.getSettingsStore()).get<boolean>(START_MINIMIZED_KEY).then((value) => value ?? true);
  }

  async setStartMinimizedOnAutostart(enabled: boolean): Promise<boolean> {
    const store = await this.getSettingsStore();
    await store.set(START_MINIMIZED_KEY, enabled);
    return enabled;
  }

  async getLowBatteryNotificationsEnabled(): Promise<boolean> {
    return (await this.getSettingsStore())
      .get<boolean>(LOW_BATTERY_NOTIFICATIONS_KEY)
      .then((value) => value ?? true);
  }

  async setLowBatteryNotificationsEnabled(enabled: boolean): Promise<boolean> {
    const store = await this.getSettingsStore();
    await store.set(LOW_BATTERY_NOTIFICATIONS_KEY, enabled);
    return enabled;
  }

  async getLowBatteryThreshold(): Promise<number> {
    return (await this.getSettingsStore())
      .get<number>(LOW_BATTERY_THRESHOLD_KEY)
      .then((value) => value ?? 20);
  }

  async setLowBatteryThreshold(value: number): Promise<number> {
    const normalized = Math.min(50, Math.max(5, Math.round(value)));
    const store = await this.getSettingsStore();
    await store.set(LOW_BATTERY_THRESHOLD_KEY, normalized);
    return normalized;
  }

  async sendLowBatteryNotification(level: number, deviceLabel: string): Promise<boolean> {
    try {
      let permissionGranted = await isPermissionGranted();

      if (!permissionGranted) {
        const permission = await requestPermission();
        permissionGranted = permission === 'granted';
      }

      if (!permissionGranted) {
        return false;
      }

      const localizedDeviceLabel = this.i18n.localizeDeviceLabel(deviceLabel);

      await sendNotification({
        title: this.i18n.t('notifications.lowBatteryTitle'),
        body: this.i18n.t('notifications.lowBatteryBody', {
          deviceLabel: localizedDeviceLabel,
          level,
        }),
      });

      return true;
    } catch {
      return false;
    }
  }

  async getBatteryHistory(): Promise<BatteryHistoryEntry[]> {
    return (await this.getSettingsStore())
      .get<BatteryHistoryEntry[]>(BATTERY_HISTORY_KEY)
      .then((value) => Array.isArray(value) ? value : []);
  }

  async setBatteryHistory(history: BatteryHistoryEntry[]): Promise<void> {
    const store = await this.getSettingsStore();
    await store.set(BATTERY_HISTORY_KEY, history);
  }

  private async bindBatteryEvents(): Promise<void> {
    try {
      this.unlisten = await listen<BatterySnapshot>('battery-updated', (event) => {
        this.updatesSubject.next(this.normalize(event.payload));
      });
    } catch {
      this.unlisten = null;
    }
  }

  private async bindSettingsEvents(): Promise<void> {
    try {
      await listen(SETTINGS_UPDATED_EVENT, () => {
        this.settingsUpdatesSubject.next();
      });
    } catch {
      return;
    }
  }

  private normalize(snapshot: BatterySnapshot): BatterySnapshot {
    const updatedAt = this.normalizeTimestamp(snapshot.updatedAt);
    const diagnostics = snapshot.diagnostics ?? {
      selectedCandidate: null,
      candidateCount: 0,
      candidates: [],
      lastError: null,
      backend: snapshot.source ?? 'browser-preview',
    };

    return {
      ...snapshot,
      level: this.clamp(snapshot.level),
      charge: Math.max(0, Number(snapshot.charge ?? 0)),
      voltage: Number(snapshot.voltage ?? 0),
      updatedAt,
      source: snapshot.source ?? 'browser-preview',
      deviceLabel: snapshot.deviceLabel ?? 'ATK device',
      status: snapshot.status ?? this.i18n.t('battery.noInfoStatus'),
      connected: Boolean(snapshot.connected),
      isCharging: Boolean(snapshot.isCharging),
      diagnostics: {
        selectedCandidate: diagnostics.selectedCandidate ?? null,
        candidateCount: Math.max(0, Number(diagnostics.candidateCount ?? diagnostics.candidates?.length ?? 0)),
        candidates: Array.isArray(diagnostics.candidates) ? diagnostics.candidates : [],
        lastError: diagnostics.lastError ?? null,
        backend: diagnostics.backend ?? snapshot.source ?? 'browser-preview',
      },
    };
  }

  private createPreviewSnapshot(): BatterySnapshot {
    return {
      level: 78,
      charge: 0,
      voltage: 3.9,
      isCharging: false,
      connected: true,
      status: this.i18n.t('battery.previewStatus'),
      deviceLabel: 'ATK device preview',
      updatedAt: new Date().toISOString(),
      source: 'browser-preview',
      diagnostics: {
        selectedCandidate: 'ATK device preview',
        candidateCount: 1,
        candidates: [
          {
            vendorId: 0x373b,
            productId: 0x1031,
            usagePage: 0xff00,
            usage: 0x0001,
            label: 'ATK device preview [373B:1031 uFF00:0001]',
            score: 160,
          },
        ],
        lastError: null,
        backend: 'browser-preview',
      },
    };
  }

  private clamp(value: number): number {
    return Math.min(100, Math.max(0, Math.round(Number(value ?? 0))));
  }

  private async getSettingsStore(): Promise<Store> {
    if (!this.settingsStorePromise) {
      this.settingsStorePromise = Store.load(SETTINGS_FILE, {
        autoSave: 100,
        defaults: {
          [START_MINIMIZED_KEY]: true,
          [LOW_BATTERY_NOTIFICATIONS_KEY]: true,
          [LOW_BATTERY_THRESHOLD_KEY]: 20,
          [BATTERY_HISTORY_KEY]: [],
        },
      });
    }

    return this.settingsStorePromise;
  }

  private normalizeTimestamp(value?: string): string {
    if (!value) {
      return new Date().toISOString();
    }

    if (/^\d+$/.test(value)) {
      return new Date(Number(value) * 1000).toISOString();
    }

    return value;
  }
}