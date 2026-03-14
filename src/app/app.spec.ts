import { ReplaySubject, Subject } from 'rxjs';
import { TestBed } from '@angular/core/testing';
import { App } from './app';
import { BatteryService } from './battery.service';
import type { BatterySnapshot } from './battery.models';

const previewSnapshot: BatterySnapshot = {
  level: 78,
  charge: 0,
  voltage: 3.9,
  isCharging: false,
  connected: true,
  status: 'Browser preview mode. Launch Tauri to read the real battery.',
  deviceLabel: 'ATK device preview',
  updatedAt: '2026-03-14T10:00:00.000Z',
  source: 'browser-preview',
  diagnostics: {
    selectedCandidate: 'ATK device preview',
    candidateCount: 1,
    candidates: [],
    lastError: null,
    backend: 'browser-preview',
  },
};

function createBatteryServiceStub(): Pick<
  BatteryService,
  | 'batteryUpdates$'
  | 'settingsUpdates$'
  | 'refreshBattery'
  | 'getBatteryHistory'
  | 'getLaunchOnStartup'
  | 'getStartMinimizedOnAutostart'
  | 'getLowBatteryNotificationsEnabled'
  | 'getLowBatteryThreshold'
  | 'fitWindowToContent'
  | 'setBatteryHistory'
  | 'sendLowBatteryNotification'
> {
  return {
    batteryUpdates$: new ReplaySubject<BatterySnapshot>(1).asObservable(),
    settingsUpdates$: new Subject<void>().asObservable(),
    refreshBattery: async () => previewSnapshot,
    getBatteryHistory: async () => [],
    getLaunchOnStartup: async () => false,
    getStartMinimizedOnAutostart: async () => true,
    getLowBatteryNotificationsEnabled: async () => true,
    getLowBatteryThreshold: async () => 20,
    fitWindowToContent: async () => undefined,
    setBatteryHistory: async () => undefined,
    sendLowBatteryNotification: async () => false,
  };
}

describe('App', () => {
  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [App],
      providers: [
        {
          provide: BatteryService,
          useValue: createBatteryServiceStub(),
        },
      ],
    }).compileComponents();
  });

  it('should create the app', () => {
    const fixture = TestBed.createComponent(App);
    const app = fixture.componentInstance;
    expect(app).toBeTruthy();
  });

  it('should render title', async () => {
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.querySelector('h1')?.textContent).toContain('ATK DEVICE PREVIEW');
  });
});
