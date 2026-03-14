export interface BatteryDiagnosticCandidate {
  vendorId: number;
  productId: number;
  usagePage: number;
  usage: number;
  label: string;
  score: number;
}

export interface BatteryDiagnostics {
  selectedCandidate: string | null;
  candidateCount: number;
  candidates: BatteryDiagnosticCandidate[];
  lastError: string | null;
  backend: string;
}

export interface BatteryHistoryEntry {
  level: number;
  updatedAt: string;
}

export interface BatterySnapshot {
  level: number;
  charge: number;
  voltage: number;
  isCharging: boolean;
  connected: boolean;
  status: string;
  deviceLabel: string;
  updatedAt: string;
  source: string;
  diagnostics: BatteryDiagnostics;
}