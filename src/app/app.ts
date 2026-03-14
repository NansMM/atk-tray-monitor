import { NgClass } from '@angular/common';
import { ChangeDetectionStrategy, Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { interval } from 'rxjs';
import { BatteryService } from './battery.service';
import type { BatteryHistoryEntry, BatterySnapshot } from './battery.models';

type BatteryHistoryPoint = BatteryHistoryEntry & {
  x: number;
  y: number;
  timeLabel: string;
};

@Component({
  selector: 'app-root',
  imports: [NgClass],
  templateUrl: './app.html',
  styleUrl: './app.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class App {
  private static readonly HISTORY_LIMIT = 10;
  private static readonly REFRESH_INTERVAL_MS = 20_000;
  private readonly batteryService = inject(BatteryService);
  private readonly destroyRef = inject(DestroyRef);
  private readonly ringLength = 339.292;

  protected readonly battery = signal<BatterySnapshot | null>(null);
  protected readonly batteryHistory = signal<BatteryHistoryEntry[]>([]);
  protected readonly hoveredHistoryIndex = signal<number | null>(null);
  protected readonly loading = signal(true);
  protected readonly syncing = signal(false);
  protected readonly startupBusy = signal(false);
  protected readonly launchOnStartup = signal(false);
  protected readonly startMinimizedOnAutostart = signal(true);
  protected readonly notificationBusy = signal(false);
  protected readonly lowBatteryNotifications = signal(true);
  protected readonly lowBatteryThreshold = signal(20);
  protected readonly errorMessage = signal<string | null>(null);
  private lastNotifiedLevel: number | null = null;

  protected readonly percentage = computed(() => this.battery()?.level ?? 0);
  protected readonly deviceTitle = computed(() => {
    const snapshot = this.battery();
    const deviceLabel = this.formatDeviceTitle(snapshot?.deviceLabel);

    if (snapshot?.connected && deviceLabel) {
      return deviceLabel;
    }

    return 'Batterie F1';
  });
  protected readonly ringOffset = computed(
    () => this.ringLength - (this.ringLength * this.percentage()) / 100,
  );
  protected readonly historyPath = computed(() => {
    const history = this.historyPoints();

    if (history.length < 2) {
      return '';
    }

    return history
      .map((entry, index) => {
        return `${index === 0 ? 'M' : 'L'} ${entry.x.toFixed(1)} ${entry.y.toFixed(1)}`;
      })
      .join(' ');
  });
  protected readonly historyPoints = computed<BatteryHistoryPoint[]>(() => {
    const history = this.batteryHistory();

    if (!history.length) {
      return [];
    }

    const width = 240;
    const height = 54;
    const step = history.length > 1 ? width / (history.length - 1) : width;

    return history.map((entry, index) => ({
      ...entry,
      x: index * step,
      y: height - (Math.max(0, Math.min(100, entry.level)) / 100) * height,
      timeLabel: this.formatTime(entry.updatedAt),
    }));
  });
  protected readonly historyLabels = computed(() => {
    const history = this.batteryHistory();

    if (!history.length) {
      return { first: '--:--', last: '--:--' };
    }

    return {
      first: this.formatTime(history[0].updatedAt),
      last: this.formatTime(history.at(-1)?.updatedAt),
    };
  });
  protected readonly historyLegend = computed(() => {
    const history = this.batteryHistory();

    if (!history.length) {
      return { min: '--', max: '--', latest: '--' };
    }

    const levels = history.map((entry) => entry.level);

    return {
      min: `${Math.min(...levels)}%`,
      max: `${Math.max(...levels)}%`,
      latest: `${history.at(-1)?.level ?? 0}%`,
    };
  });
  protected readonly hoveredHistoryPoint = computed(() => {
    const hoveredIndex = this.hoveredHistoryIndex();

    if (hoveredIndex === null) {
      return null;
    }

    return this.historyPoints()[hoveredIndex] ?? null;
  });
  protected readonly previewMode = computed(() => this.battery()?.source === 'browser-preview');
  protected readonly connectionLabel = computed(() => {
    const snapshot = this.battery();

    if (!snapshot) {
      return 'Connexion';
    }

    if (snapshot.source === 'browser-preview') {
      return 'Preview';
    }

    if (!snapshot.connected) {
      return 'Hors ligne';
    }

    return snapshot.isCharging ? 'Charge' : 'Batterie';
  });
  protected readonly statusTagTone = computed(() => {
    const snapshot = this.battery();

    if (!snapshot || snapshot.source === 'browser-preview') {
      return 'border-amber-400/40 bg-amber-400/10 text-amber-200';
    }

    if (!snapshot.connected) {
      return 'border-rose-400/30 bg-rose-400/10 text-rose-200';
    }

    return snapshot.isCharging
      ? 'border-cyan-400/30 bg-cyan-400/10 text-cyan-200'
      : 'border-emerald-400/30 bg-emerald-400/10 text-emerald-200';
  });
  protected readonly statusTone = computed(() => {
    const snapshot = this.battery();

    if (!snapshot) {
      return 'text-cyan-100';
    }

    if (snapshot.level <= 20) {
      return 'text-amber-300';
    }

    return snapshot.connected ? 'text-emerald-300' : 'text-rose-300';
  });

  constructor() {
    this.batteryService.batteryUpdates$
      .pipe(takeUntilDestroyed(this.destroyRef))
      .subscribe((snapshot) => {
        this.applySnapshot(snapshot);
        this.loading.set(false);
        this.errorMessage.set(snapshot.connected ? null : snapshot.status);
        void this.maybeNotifyLowBattery(snapshot);
      });

    this.batteryService.settingsUpdates$
      .pipe(takeUntilDestroyed(this.destroyRef))
      .subscribe(() => {
        void this.loadStartupPreference();
        void this.loadNotificationPreferences();
      });

    interval(App.REFRESH_INTERVAL_MS)
      .pipe(takeUntilDestroyed(this.destroyRef))
      .subscribe(() => {
        void this.refresh(false);
      });

    void this.refresh(true);
    void this.loadBatteryHistory();
    void this.loadStartupPreference();
    void this.loadNotificationPreferences();
  }

  protected async refresh(showLoader: boolean): Promise<void> {
    if (showLoader) {
      this.loading.set(true);
    }

    this.syncing.set(true);

    try {
      const snapshot = await this.batteryService.refreshBattery();
      this.applySnapshot(snapshot);
      this.errorMessage.set(snapshot.connected ? null : snapshot.status);
    } catch (error) {
      this.errorMessage.set(error instanceof Error ? error.message : 'Echec de la synchronisation.');
    } finally {
      this.loading.set(false);
      this.syncing.set(false);
    }
  }

  protected async hideToTray(): Promise<void> {
    await this.batteryService.hideWindow();
  }

  protected async toggleLaunchOnStartup(): Promise<void> {
    this.startupBusy.set(true);

    try {
      const enabled = await this.batteryService.setLaunchOnStartup(!this.launchOnStartup());
      this.launchOnStartup.set(enabled);
    } finally {
      this.startupBusy.set(false);
    }
  }

  protected async toggleStartMinimizedOnAutostart(): Promise<void> {
    this.startupBusy.set(true);

    try {
      const enabled = await this.batteryService.setStartMinimizedOnAutostart(
        !this.startMinimizedOnAutostart(),
      );
      this.startMinimizedOnAutostart.set(enabled);
    } finally {
      this.startupBusy.set(false);
    }
  }

  protected async toggleLowBatteryNotifications(): Promise<void> {
    this.notificationBusy.set(true);

    try {
      const enabled = await this.batteryService.setLowBatteryNotificationsEnabled(
        !this.lowBatteryNotifications(),
      );
      this.lowBatteryNotifications.set(enabled);
      if (!enabled) {
        this.lastNotifiedLevel = null;
      }
    } finally {
      this.notificationBusy.set(false);
    }
  }

  protected async setLowBatteryThreshold(value: number): Promise<void> {
    const threshold = await this.batteryService.setLowBatteryThreshold(value);
    this.lowBatteryThreshold.set(threshold);
    this.lastNotifiedLevel = null;
  }

  protected setHoveredHistoryIndex(index: number | null): void {
    this.hoveredHistoryIndex.set(index);
  }

  protected trackHistoryPoint(_index: number, point: BatteryHistoryPoint): string {
    return point.updatedAt;
  }

  private async loadStartupPreference(): Promise<void> {
    const enabled = await this.batteryService.getLaunchOnStartup();
    this.launchOnStartup.set(enabled);
    const startMinimized = await this.batteryService.getStartMinimizedOnAutostart();
    this.startMinimizedOnAutostart.set(startMinimized);
  }

  private formatDeviceTitle(label: string | null | undefined): string {
    if (!label) {
      return '';
    }

    const cleaned = label
      .replace(/\s*\[[^\]]+\]\s*$/u, '')
      .replace(/\s+/gu, ' ')
      .trim();

    if (!cleaned) {
      return '';
    }

    return cleaned.toUpperCase();
  }

  private async loadBatteryHistory(): Promise<void> {
    const history = await this.batteryService.getBatteryHistory();
    this.batteryHistory.set(history.slice(-App.HISTORY_LIMIT));
  }

  private async loadNotificationPreferences(): Promise<void> {
    const enabled = await this.batteryService.getLowBatteryNotificationsEnabled();
    const threshold = await this.batteryService.getLowBatteryThreshold();

    this.lowBatteryNotifications.set(enabled);
    this.lowBatteryThreshold.set(threshold);
  }

  private async maybeNotifyLowBattery(snapshot: BatterySnapshot): Promise<void> {
    if (
      snapshot.source === 'browser-preview' ||
      !snapshot.connected ||
      snapshot.isCharging ||
      !this.lowBatteryNotifications()
    ) {
      this.lastNotifiedLevel = null;
      return;
    }

    if (snapshot.level > this.lowBatteryThreshold()) {
      this.lastNotifiedLevel = null;
      return;
    }

    if (this.lastNotifiedLevel === snapshot.level) {
      return;
    }

    const sent = await this.batteryService.sendLowBatteryNotification(
      snapshot.level,
      snapshot.deviceLabel,
    );

    if (sent) {
      this.lastNotifiedLevel = snapshot.level;
    }
  }

  private applySnapshot(snapshot: BatterySnapshot): void {
    this.battery.set(snapshot);

    if (!snapshot.connected || snapshot.source === 'browser-preview') {
      return;
    }

    const history = this.batteryHistory();

    if (history.at(-1)?.updatedAt === snapshot.updatedAt) {
      return;
    }

    const nextHistory = [
      ...history,
      {
        level: snapshot.level,
        updatedAt: snapshot.updatedAt,
      },
    ].slice(-App.HISTORY_LIMIT);

    this.batteryHistory.set(nextHistory);

    void this.batteryService.setBatteryHistory(nextHistory);
  }

  private formatTime(value?: string): string {
    if (!value) {
      return '--:--';
    }

    const date = new Date(value);

    if (Number.isNaN(date.getTime())) {
      return '--:--';
    }

    return date.toLocaleTimeString('fr-FR', {
      hour: '2-digit',
      minute: '2-digit',
    });
  }
}
