import { Injectable, inject } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { I18nService } from './i18n.service';

const INITIAL_UPDATE_DELAY_MS = 5_000;
const BACKGROUND_UPDATE_INTERVAL_MS = 6 * 60 * 60 * 1000;
const RESTART_DELAY_MS = 1_500;

@Injectable({ providedIn: 'root' })
export class UpdateService {
  private readonly i18n = inject(I18nService);
  private started = false;
  private activeCheck: Promise<void> | null = null;
  private intervalId: number | null = null;
  private initialTimerId: number | null = null;

  start(): void {
    if (this.started || typeof window === 'undefined') {
      return;
    }

    this.started = true;
    this.initialTimerId = window.setTimeout(() => {
      this.initialTimerId = null;
      void this.checkForUpdates();
    }, INITIAL_UPDATE_DELAY_MS);
    this.intervalId = window.setInterval(() => {
      void this.checkForUpdates();
    }, BACKGROUND_UPDATE_INTERVAL_MS);
  }

  private async checkForUpdates(): Promise<void> {
    if (this.activeCheck) {
      return this.activeCheck;
    }

    this.activeCheck = this.runUpdateCheck().finally(() => {
      this.activeCheck = null;
    });

    return this.activeCheck;
  }

  private async runUpdateCheck(): Promise<void> {
    try {
      const version = await invoke<string | null>('install_available_update');

      if (!version) {
        return;
      }

      await this.notify(
        this.i18n.t('notifications.updateReadyTitle'),
        this.i18n.t('notifications.updateInstalledBody', {
          version,
        }),
      );

      await this.delay(RESTART_DELAY_MS);

      await invoke('restart_app');
    } catch {
      return;
    }
  }

  private async delay(durationMs: number): Promise<void> {
    await new Promise<void>((resolve) => {
      window.setTimeout(resolve, durationMs);
    });
  }

  private async notify(title: string, body: string): Promise<void> {
    try {
      let permissionGranted = await isPermissionGranted();

      if (!permissionGranted) {
        const permission = await requestPermission();
        permissionGranted = permission === 'granted';
      }

      if (!permissionGranted) {
        return;
      }

      await sendNotification({
        title,
        body,
      });
    } catch {
      return;
    }
  }
}