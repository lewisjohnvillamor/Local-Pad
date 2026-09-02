// Per-phone controller preferences, persisted locally on the device.

export interface ControllerSettings {
  /// Touchpad gain multiplier.
  pointerSpeed: number;
  /// Scroll gain multiplier.
  scrollSpeed: number;
  /// When false, scrolling direction is inverted.
  naturalScroll: boolean;
  /// Vibrate briefly on button presses where the browser allows it.
  haptics: boolean;
}

export const DEFAULT_SETTINGS: ControllerSettings = {
  pointerSpeed: 1,
  scrollSpeed: 1,
  naturalScroll: true,
  haptics: true,
};

const KEY = "localpad-settings";

export function loadSettings(): ControllerSettings {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULT_SETTINGS };
    const parsed = JSON.parse(raw) as Partial<ControllerSettings>;
    return {
      pointerSpeed: clamp(parsed.pointerSpeed, 0.4, 3, 1),
      scrollSpeed: clamp(parsed.scrollSpeed, 0.4, 3, 1),
      naturalScroll: parsed.naturalScroll ?? true,
      haptics: parsed.haptics ?? true,
    };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export function saveSettings(settings: ControllerSettings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(settings));
  } catch {
    // Private browsing: settings simply do not persist.
  }
}

function clamp(v: number | undefined, min: number, max: number, fallback: number): number {
  if (typeof v !== "number" || !Number.isFinite(v)) return fallback;
  return Math.min(max, Math.max(min, v));
}

/// Stable per-phone identifier so the server replaces this device on
/// re-pairing instead of listing it twice.
export function deviceUid(): string | undefined {
  try {
    const existing = localStorage.getItem("localpad-uid");
    if (existing) return existing;
    const fresh = crypto.randomUUID();
    localStorage.setItem("localpad-uid", fresh);
    return fresh;
  } catch {
    return undefined;
  }
}
