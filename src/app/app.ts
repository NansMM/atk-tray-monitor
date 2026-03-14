import { NgClass } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  afterNextRender,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { interval } from 'rxjs';
import { BatteryService } from './battery.service';
import type { BatteryHistoryEntry, BatterySnapshot } from './battery.models';
import { I18nService } from './i18n.service';
import { UpdateService } from './update.service';

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
  private static readonly HISTORY_WINDOW_MS = 60 * 60 * 1000;
  private static readonly HISTORY_BUCKET_MS = Math.floor(
    App.HISTORY_WINDOW_MS / App.HISTORY_LIMIT,
  );
  private static readonly REFRESH_INTERVAL_MS = 20_000;
  private readonly batteryService = inject(BatteryService);
  private readonly destroyRef = inject(DestroyRef);
  protected readonly i18n = inject(I18nService);
  private readonly updateService = inject(UpdateService);
  private readonly contentRoot = viewChild.required<ElementRef<HTMLElement>>('contentRoot');
  private readonly ringLength = 339.292;
  private lastSyncedWindowHeight = 0;
  private resizeFrameId: number | null = null;

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

    return this.i18n.t('battery.fallbackDeviceTitle');
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
      return this.i18n.t('status.connecting');
    }

    if (snapshot.source === 'browser-preview') {
      return this.i18n.t('status.preview');
    }

    if (!snapshot.connected) {
      return this.i18n.t('status.offline');
    }

    return snapshot.isCharging
      ? this.i18n.t('status.charging')
      : this.i18n.t('status.battery');
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
    this.updateService.start();

    afterNextRender(() => {
      this.bindWindowHeightSync();
    });
  }

  private bindWindowHeightSync(): void {
    const contentRoot = this.contentRoot().nativeElement;

    this.scheduleWindowHeightSync();

    if (typeof ResizeObserver === 'undefined') {
      this.destroyRef.onDestroy(() => {
        this.cancelScheduledWindowHeightSync();
      });
      return;
    }

    const resizeObserver = new ResizeObserver(() => {
      this.scheduleWindowHeightSync();
    });

    resizeObserver.observe(contentRoot);

    this.destroyRef.onDestroy(() => {
      resizeObserver.disconnect();
      this.cancelScheduledWindowHeightSync();
    });
  }

  private scheduleWindowHeightSync(): void {
    if (typeof window === 'undefined') {
      return;
    }

    this.cancelScheduledWindowHeightSync();

    const syncWindowHeight = () => {
      this.resizeFrameId = null;
      const contentHeight = Math.ceil(
        this.contentRoot().nativeElement.getBoundingClientRect().height,
      );

      if (Math.abs(contentHeight - this.lastSyncedWindowHeight) <= 1) {
        return;
      }

      this.lastSyncedWindowHeight = contentHeight;
      void this.batteryService.fitWindowToContent(contentHeight);
    };

    if (typeof window.requestAnimationFrame !== 'function') {
      syncWindowHeight();
      return;
    }

    this.resizeFrameId = window.requestAnimationFrame(syncWindowHeight);
  }

  private cancelScheduledWindowHeightSync(): void {
    if (typeof window === 'undefined' || this.resizeFrameId === null) {
      return;
    }

    if (typeof window.cancelAnimationFrame === 'function') {
      window.cancelAnimationFrame(this.resizeFrameId);
    }

    this.resizeFrameId = null;
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
      this.errorMessage.set(
        error instanceof Error
          ? error.message
          : this.i18n.t('errors.syncFailed'),
      );
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

    const localized = this.i18n.localizeDeviceLabel(label);

    const cleaned = localized
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
    const normalizedHistory = this.normalizeHistory(history);

    this.batteryHistory.set(normalizedHistory);

    if (JSON.stringify(normalizedHistory) !== JSON.stringify(history)) {
      void this.batteryService.setBatteryHistory(normalizedHistory);
    }
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

    const nextHistory = this.normalizeHistory(
      [
        ...history,
        {
          level: snapshot.level,
          updatedAt: snapshot.updatedAt,
        },
      ],
      this.toTimestamp(snapshot.updatedAt) ?? Date.now(),
    );

    this.batteryHistory.set(nextHistory);

    void this.batteryService.setBatteryHistory(nextHistory);
  }

  private normalizeHistory(
    history: BatteryHistoryEntry[],
    referenceTimeMs = Date.now(),
  ): BatteryHistoryEntry[] {
    const cutoffTimeMs = referenceTimeMs - App.HISTORY_WINDOW_MS;
    const compacted = history
      .map((entry) => ({
        entry: {
          level: Math.max(0, Math.min(100, Math.round(entry.level))),
          updatedAt: entry.updatedAt,
        },
        timestamp: this.toTimestamp(entry.updatedAt),
      }))
      .filter(
        (
          item,
        ): item is { entry: BatteryHistoryEntry; timestamp: number } => item.timestamp !== null,
      )
      .filter((item) => item.timestamp >= cutoffTimeMs)
      .sort((left, right) => left.timestamp - right.timestamp)
      .reduce<Array<{ entry: BatteryHistoryEntry; bucket: number }>>((result, item) => {
        const bucket = this.historyBucketFor(item.timestamp, cutoffTimeMs);
        const previous = result.at(-1);

        if (previous?.bucket === bucket) {
          previous.entry = item.entry;
          return result;
        }

        result.push({
          entry: item.entry,
          bucket,
        });
        return result;
      }, []);

    return compacted.slice(-App.HISTORY_LIMIT).map((item) => item.entry);
  }

  private historyBucketFor(timestampMs: number, cutoffTimeMs: number): number {
    const elapsed = Math.max(0, timestampMs - cutoffTimeMs);
    return Math.min(
      App.HISTORY_LIMIT - 1,
      Math.floor(elapsed / App.HISTORY_BUCKET_MS),
    );
  }

  private toTimestamp(value: string): number | null {
    const timestamp = Date.parse(value);

    return Number.isNaN(timestamp) ? null : timestamp;
  }

  protected historyPointLabel(level: number, timeLabel: string): string {
    return this.i18n.t('history.pointLabel', {
      level,
      time: timeLabel,
    });
  }

  protected historyLegendLabel(kind: 'min' | 'max' | 'current', value: string): string {
    switch (kind) {
      case 'min':
        return this.i18n.t('history.min', { value });
      case 'max':
        return this.i18n.t('history.max', { value });
      default:
        return this.i18n.t('history.current', { value });
    }
  }

  private formatTime(value?: string): string {
    if (!value) {
      return '--:--';
    }

    const date = new Date(value);

    if (Number.isNaN(date.getTime())) {
      return '--:--';
    }

    return date.toLocaleTimeString(this.i18n.locale(), {
      hour: '2-digit',
      minute: '2-digit',
    });
  }
}
