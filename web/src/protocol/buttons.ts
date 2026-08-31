// Gamepad button bit assignments; mirrors GamepadButton in
// crates/localpad-core/src/frame.rs.

export const BUTTON_BITS: Record<string, number> = {
  dpad_up: 1 << 0,
  dpad_down: 1 << 1,
  dpad_left: 1 << 2,
  dpad_right: 1 << 3,
  a: 1 << 4,
  b: 1 << 5,
  x: 1 << 6,
  y: 1 << 7,
  start: 1 << 8,
  select: 1 << 9,
  l1: 1 << 10,
  r1: 1 << 11,
  l2: 1 << 12,
  r2: 1 << 13,
  l3: 1 << 14,
  r3: 1 << 15,
  guide: 1 << 16,
};

export const BUTTON_NAMES = Object.keys(BUTTON_BITS);
