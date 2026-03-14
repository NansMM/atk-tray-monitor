import { Injectable, computed, signal } from '@angular/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Store } from '@tauri-apps/plugin-store';

import de from './i18n/de.json';
import en from './i18n/en.json';
import es from './i18n/es.json';
import fr from './i18n/fr.json';
import it from './i18n/it.json';

export type SupportedLanguage = 'de' | 'en' | 'es' | 'fr' | 'it';

type TranslationValue = string | TranslationTree;
interface TranslationTree {
  [key: string]: TranslationValue;
}
type TranslationParams = Record<string, string | number | boolean | null | undefined>;

const STORAGE_KEY = 'atk-tray-monitor.language';
const SETTINGS_FILE = 'settings.json';
const SETTINGS_UPDATED_EVENT = 'settings-updated';
const LANGUAGE_KEY = 'language';
const DEFAULT_LANGUAGE: SupportedLanguage = 'en';
const FALLBACK_LANGUAGE: SupportedLanguage = 'en';
const SUPPORTED_LANGUAGES = ['de', 'en', 'es', 'fr', 'it'] as const;
const TRANSLATIONS: Record<SupportedLanguage, TranslationTree> = {
  de,
  en,
  es,
  fr,
  it,
};

@Injectable({ providedIn: 'root' })
export class I18nService {
  readonly languages = [
    { code: 'de', label: 'DE' },
    { code: 'en', label: 'EN' },
    { code: 'es', label: 'ES' },
    { code: 'fr', label: 'FR' },
    { code: 'it', label: 'IT' },
  ] as const;

  private readonly activeLanguage = signal<SupportedLanguage>(this.resolveInitialLanguage());
  private settingsStorePromise: Promise<Store> | null = null;
  private settingsUnlisten: UnlistenFn | null = null;
  readonly currentLanguage = this.activeLanguage.asReadonly();
  readonly locale = computed(() => this.toLocale(this.activeLanguage()));

  constructor() {
    this.applyDocumentLanguage(this.activeLanguage());
    void this.syncLanguageFromSettings();
    void this.bindSettingsEvents();
  }

  setLanguage(language: SupportedLanguage): void {
    if (!this.isSupportedLanguage(language) || language === this.activeLanguage()) {
      return;
    }

    this.activeLanguage.set(language);
    this.applyDocumentLanguage(language);
    this.persistLanguage(language);
  }

  t(key: string, params: TranslationParams = {}): string {
    const language = this.activeLanguage();
    const value = this.lookup(TRANSLATIONS[language], key) ?? this.lookup(TRANSLATIONS[FALLBACK_LANGUAGE], key);

    if (typeof value !== 'string') {
      return key;
    }

    return value.replace(/\{\{\s*(\w+)\s*\}\}/g, (_match, paramName: string) => {
      const paramValue = params[paramName];
      return paramValue === undefined || paramValue === null ? '' : String(paramValue);
    });
  }

  localizeDeviceLabel(label: string | null | undefined): string {
    if (!label) {
      return '';
    }

    const normalized = label.trim().toLowerCase();

    if (normalized === 'atk mouse') {
      return this.t('battery.genericMouseTitle');
    }

    return label;
  }

  private resolveInitialLanguage(): SupportedLanguage {
    const storedLanguage = this.readStoredLanguage();

    if (storedLanguage) {
      return storedLanguage;
    }

    return this.detectSystemLanguage();
  }

  private readStoredLanguage(): SupportedLanguage | null {
    try {
      const value = globalThis.localStorage?.getItem(STORAGE_KEY);
      return this.isSupportedLanguage(value) ? value : null;
    } catch {
      return null;
    }
  }

  private persistLanguage(language: SupportedLanguage): void {
    try {
      globalThis.localStorage?.setItem(STORAGE_KEY, language);
    } catch {
      // Ignore storage errors in preview/test contexts.
    }

    void this.saveLanguageToSettings(language);
  }

  private async syncLanguageFromSettings(): Promise<void> {
    const storedLanguage = await this.readLanguageFromSettings();

    if (storedLanguage) {
      this.applyLanguage(storedLanguage, false);
      return;
    }

    await this.saveLanguageToSettings(this.activeLanguage());
  }

  private async bindSettingsEvents(): Promise<void> {
    try {
      this.settingsUnlisten = await listen(SETTINGS_UPDATED_EVENT, async () => {
        const storedLanguage = await this.readLanguageFromSettings();

        if (storedLanguage && storedLanguage !== this.activeLanguage()) {
          this.applyLanguage(storedLanguage, false);
        }
      });
    } catch {
      this.settingsUnlisten = null;
    }
  }

  private detectSystemLanguage(): SupportedLanguage {
    const candidates = [
      ...(globalThis.navigator?.languages ?? []),
      globalThis.navigator?.language,
      Intl.DateTimeFormat().resolvedOptions().locale,
    ].filter((value): value is string => typeof value === 'string' && value.length > 0);

    for (const candidate of candidates) {
      const normalized = candidate.toLowerCase();

      if (normalized.startsWith('fr')) {
        return 'fr';
      }

      if (normalized.startsWith('de')) {
        return 'de';
      }

      if (normalized.startsWith('en')) {
        return 'en';
      }

      if (normalized.startsWith('es')) {
        return 'es';
      }

      if (normalized.startsWith('it')) {
        return 'it';
      }
    }

    return DEFAULT_LANGUAGE;
  }

  private lookup(tree: TranslationTree, key: string): TranslationValue | null {
    let current: TranslationValue = tree;

    for (const segment of key.split('.')) {
      if (typeof current !== 'object' || current === null || !(segment in current)) {
        return null;
      }

      current = current[segment];
    }

    return current;
  }

  private toLocale(language: SupportedLanguage): string {
    switch (language) {
      case 'de':
        return 'de-DE';
      case 'es':
        return 'es-ES';
      case 'fr':
        return 'fr-FR';
      case 'it':
        return 'it-IT';
      default:
        return 'en-US';
    }
  }

  private applyDocumentLanguage(language: SupportedLanguage): void {
    if (typeof document === 'undefined') {
      return;
    }

    document.documentElement.lang = language;
  }

  private applyLanguage(language: SupportedLanguage, persist: boolean): void {
    this.activeLanguage.set(language);
    this.applyDocumentLanguage(language);

    try {
      globalThis.localStorage?.setItem(STORAGE_KEY, language);
    } catch {
      // Ignore storage errors in preview/test contexts.
    }

    if (persist) {
      void this.saveLanguageToSettings(language);
    }
  }

  private async getSettingsStore(): Promise<Store> {
    if (!this.settingsStorePromise) {
      this.settingsStorePromise = Store.load(SETTINGS_FILE, {
        autoSave: 100,
        defaults: {
          [LANGUAGE_KEY]: this.activeLanguage(),
        },
      });
    }

    return this.settingsStorePromise;
  }

  private async readLanguageFromSettings(): Promise<SupportedLanguage | null> {
    try {
      const store = await this.getSettingsStore();
      const value = await store.get<string>(LANGUAGE_KEY);
      return this.isSupportedLanguage(value) ? value : null;
    } catch {
      return null;
    }
  }

  private async saveLanguageToSettings(language: SupportedLanguage): Promise<void> {
    try {
      const store = await this.getSettingsStore();
      await store.set(LANGUAGE_KEY, language);
    } catch {
      // Ignore store errors when Tauri plugins are unavailable.
    }
  }

  private isSupportedLanguage(language: unknown): language is SupportedLanguage {
    return typeof language === 'string' && SUPPORTED_LANGUAGES.includes(language as SupportedLanguage);
  }
}