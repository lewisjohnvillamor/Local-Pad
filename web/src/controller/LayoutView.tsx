// Renders a layout's controls into the play area and wires Pointer Events
// into the session's input state. All geometry is normalized: x/y are
// fractions of the play area, size is a fraction of its shorter edge.

import React, { useEffect, useRef, useState } from "react";
import type { Control, Layout } from "../protocol/messages";
import { BUTTON_BITS } from "../protocol/buttons";
import type { ControllerSession } from "./session";

const DPAD_MASK =
  BUTTON_BITS.dpad_up | BUTTON_BITS.dpad_down | BUTTON_BITS.dpad_left | BUTTON_BITS.dpad_right;

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

function controlBox(control: Control, area: { w: number; h: number }): Box {
  const base = control.size * Math.min(area.w, area.h);
  const width = control.width != null ? control.width * area.w : base;
  const height = control.height != null ? control.height * area.h : base;
  return {
    left: control.x * area.w,
    top: control.y * area.h,
    width,
    height,
  };
}

export function LayoutView({
  layout,
  session,
  motionOn,
}: {
  layout: Layout;
  session: ControllerSession;
  motionOn: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [area, setArea] = useState({ w: 0, h: 0 });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver(() => {
      setArea({ w: el.clientWidth, h: el.clientHeight });
    });
    observer.observe(el);
    setArea({ w: el.clientWidth, h: el.clientHeight });
    return () => observer.disconnect();
  }, []);

  return (
    <div ref={ref} style={{ position: "absolute", inset: 0 }}>
      {area.w > 0 &&
        layout.controls.map((control, index) => {
          const box = controlBox(control, area);
          const key = control.id ?? `${control.type}-${index}`;
          switch (control.type) {
            case "button":
              return (
                <ButtonControl
                  key={key}
                  control={control}
                  box={box}
                  layout={layout}
                  session={session}
                />
              );
            case "dpad":
              return <DpadControl key={key} box={box} session={session} />;
            case "stick":
              return <StickControl key={key} control={control} box={box} session={session} />;
            case "touchpad":
              return <TouchpadControl key={key} box={box} session={session} />;
            case "scroll":
              return <ScrollControl key={key} box={box} session={session} />;
            case "trigger":
              return <TriggerControl key={key} control={control} box={box} session={session} />;
            case "keyboard":
              return <KeyboardControl key={key} box={box} session={session} />;
            case "motion":
              return (
                <MotionControl
                  key={key}
                  control={control}
                  box={box}
                  on={motionOn}
                  session={session}
                />
              );
            default:
              return null;
          }
        })}
    </div>
  );
}

function boxStyle(box: Box): React.CSSProperties {
  return {
    left: box.left,
    top: box.top,
    width: box.width,
    height: box.height,
  };
}

// ---- button ----

type ButtonAction =
  | { kind: "bit"; bit: number }
  | { kind: "mouse"; button: "left" | "right" | "middle" }
  | { kind: "key"; code: string }
  | { kind: "none" };

function buttonAction(layout: Layout, control: Control): ButtonAction {
  const id = control.id ?? "";
  if (id in BUTTON_BITS) return { kind: "bit", bit: BUTTON_BITS[id] };
  const binding = layout.bindings[id];
  if (binding?.startsWith("mouse:")) {
    const b = binding.slice(6);
    if (b === "left" || b === "right" || b === "middle") return { kind: "mouse", button: b };
  }
  if (binding?.startsWith("keyboard:")) return { kind: "key", code: binding.slice(9) };
  return { kind: "none" };
}

function ButtonControl({
  control,
  box,
  layout,
  session,
}: {
  control: Control;
  box: Box;
  layout: Layout;
  session: ControllerSession;
}) {
  const [pressed, setPressed] = useState(false);
  const action = buttonAction(layout, control);
  const round = control.width == null && control.height == null;

  const set = (down: boolean) => {
    setPressed(down);
    switch (action.kind) {
      case "bit":
        session.setButton(action.bit, down);
        break;
      case "mouse":
        session.sendMouseButton(action.button, down);
        break;
      case "key":
        session.sendKey(action.code, down);
        break;
      case "none":
        break;
    }
  };

  return (
    <button
      className={`ctl ctl-button control-surface ${round ? "round" : ""} ${pressed ? "pressed" : ""}`}
      style={boxStyle(box)}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        set(true);
      }}
      onPointerUp={() => set(false)}
      onPointerCancel={() => set(false)}
      onContextMenu={(e) => e.preventDefault()}
    >
      {control.label ?? control.id}
    </button>
  );
}

// ---- dpad ----

function DpadControl({ box, session }: { box: Box; session: ControllerSession }) {
  const [bits, setBits] = useState(0);
  const ref = useRef<HTMLDivElement>(null);

  const apply = (next: number) => {
    setBits(next);
    session.setDpadBits(DPAD_MASK, next);
  };

  const fromPoint = (clientX: number, clientY: number): number => {
    const el = ref.current;
    if (!el) return 0;
    const rect = el.getBoundingClientRect();
    const dx = (clientX - rect.left) / rect.width - 0.5;
    const dy = (clientY - rect.top) / rect.height - 0.5;
    if (Math.hypot(dx, dy) < 0.12) return 0;
    let next = 0;
    // Eight-way: an axis engages when it carries at least 40% of the pull.
    const angle = Math.atan2(dy, dx);
    const sector = Math.round(angle / (Math.PI / 4));
    switch ((sector + 8) % 8) {
      case 0: next = BUTTON_BITS.dpad_right; break;
      case 1: next = BUTTON_BITS.dpad_right | BUTTON_BITS.dpad_down; break;
      case 2: next = BUTTON_BITS.dpad_down; break;
      case 3: next = BUTTON_BITS.dpad_down | BUTTON_BITS.dpad_left; break;
      case 4: next = BUTTON_BITS.dpad_left; break;
      case 5: next = BUTTON_BITS.dpad_left | BUTTON_BITS.dpad_up; break;
      case 6: next = BUTTON_BITS.dpad_up; break;
      case 7: next = BUTTON_BITS.dpad_up | BUTTON_BITS.dpad_right; break;
    }
    return next;
  };

  const cell = (name: string, bit: number, symbol: string) => (
    <div key={name} className={`dir ${(bits & bit) !== 0 ? "pressed" : ""}`}>
      {symbol}
    </div>
  );

  return (
    <div
      ref={ref}
      className="ctl ctl-dpad control-surface"
      style={boxStyle(box)}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        apply(fromPoint(e.clientX, e.clientY));
      }}
      onPointerMove={(e) => {
        if (e.buttons > 0 || e.pointerType === "touch") {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) {
            apply(fromPoint(e.clientX, e.clientY));
          }
        }
      }}
      onPointerUp={() => apply(0)}
      onPointerCancel={() => apply(0)}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="blank" />
      {cell("up", BUTTON_BITS.dpad_up, "▲")}
      <div className="blank" />
      {cell("left", BUTTON_BITS.dpad_left, "◀")}
      <div className="blank" />
      {cell("right", BUTTON_BITS.dpad_right, "▶")}
      <div className="blank" />
      {cell("down", BUTTON_BITS.dpad_down, "▼")}
      <div className="blank" />
    </div>
  );
}

// ---- stick ----

function StickControl({
  control,
  box,
  session,
}: {
  control: Control;
  box: Box;
  session: ControllerSession;
}) {
  const [pos, setPos] = useState<[number, number]>([0, 0]);
  const [engaged, setEngaged] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const slot = control.id === "right" ? "right" : "left";

  const apply = (x: number, y: number) => {
    setPos([x, y]);
    if (slot === "left") session.leftStick = [x, y];
    else session.rightStick = [x, y];
  };

  const fromPoint = (clientX: number, clientY: number) => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    let dx = ((clientX - rect.left) / rect.width - 0.5) * 2.4;
    let dy = ((clientY - rect.top) / rect.height - 0.5) * 2.4;
    const mag = Math.hypot(dx, dy);
    if (mag > 1) {
      dx /= mag;
      dy /= mag;
    }
    apply(dx, dy);
  };

  return (
    <div
      ref={ref}
      className={`ctl ctl-stick control-surface ${engaged ? "engaged" : ""}`}
      style={boxStyle(box)}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        setEngaged(true);
        fromPoint(e.clientX, e.clientY);
      }}
      onPointerMove={(e) => {
        if (e.currentTarget.hasPointerCapture(e.pointerId)) {
          fromPoint(e.clientX, e.clientY);
        }
      }}
      onPointerUp={() => {
        setEngaged(false);
        apply(0, 0);
      }}
      onPointerCancel={() => {
        setEngaged(false);
        apply(0, 0);
      }}
    >
      <div
        className="thumb"
        style={{ left: `${50 + pos[0] * 28}%`, top: `${50 + pos[1] * 28}%` }}
      />
    </div>
  );
}

// ---- touchpad ----

interface TrackedPointer {
  x: number;
  y: number;
  startX: number;
  startY: number;
  moved: boolean;
  startedAt: number;
}

function TouchpadControl({ box, session }: { box: Box; session: ControllerSession }) {
  const pointers = useRef(new Map<number, TrackedPointer>());
  const lastTapAt = useRef(0);
  const dragging = useRef(false);
  const twoFingerTapCandidate = useRef(false);

  const onDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    pointers.current.set(e.pointerId, {
      x: e.clientX,
      y: e.clientY,
      startX: e.clientX,
      startY: e.clientY,
      moved: false,
      startedAt: performance.now(),
    });
    if (pointers.current.size === 1) {
      // A touch shortly after a tap starts a drag with the button held.
      if (performance.now() - lastTapAt.current < 300) {
        dragging.current = true;
        session.sendMouseButton("left", true);
      }
    }
    if (pointers.current.size === 2) {
      twoFingerTapCandidate.current = true;
    }
  };

  const onMove = (e: React.PointerEvent) => {
    const tracked = pointers.current.get(e.pointerId);
    if (!tracked) return;
    const dx = e.clientX - tracked.x;
    const dy = e.clientY - tracked.y;
    tracked.x = e.clientX;
    tracked.y = e.clientY;
    if (Math.hypot(e.clientX - tracked.startX, e.clientY - tracked.startY) > 8) {
      tracked.moved = true;
      twoFingerTapCandidate.current = false;
    }
    if (pointers.current.size === 1) {
      // Speed-sensitive gain keeps slow motion precise, fast motion quick.
      const speed = Math.hypot(dx, dy);
      const gain = 1.4 + Math.min(speed / 18, 1.6);
      session.addPointer(dx * gain, dy * gain);
    } else if (pointers.current.size === 2) {
      // Two-finger scroll uses the average of both pointers; halve so both
      // fingers together do not double the distance.
      session.addScroll(dx / 2, dy / 2);
    }
  };

  const endPointer = (e: React.PointerEvent, cancelled: boolean) => {
    const tracked = pointers.current.get(e.pointerId);
    pointers.current.delete(e.pointerId);
    if (!tracked) return;
    const duration = performance.now() - tracked.startedAt;
    if (cancelled) {
      if (dragging.current && pointers.current.size === 0) {
        dragging.current = false;
        session.sendMouseButton("left", false);
      }
      return;
    }
    if (pointers.current.size === 0) {
      if (dragging.current) {
        dragging.current = false;
        session.sendMouseButton("left", false);
      } else if (!tracked.moved && duration < 250) {
        if (twoFingerTapCandidate.current) {
          session.tapMouse("right");
        } else {
          lastTapAt.current = performance.now();
          session.tapMouse("left");
        }
      }
      twoFingerTapCandidate.current = false;
    }
  };

  return (
    <div
      className="ctl ctl-touchpad control-surface"
      style={boxStyle(box)}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={(e) => endPointer(e, false)}
      onPointerCancel={(e) => endPointer(e, true)}
      onContextMenu={(e) => e.preventDefault()}
    >
      <span className="hint">tap: click. two fingers: scroll. double tap: drag</span>
    </div>
  );
}

// ---- scroll strip ----

function ScrollControl({ box, session }: { box: Box; session: ControllerSession }) {
  const lastY = useRef<number | null>(null);
  return (
    <div
      className="ctl ctl-scroll control-surface"
      style={boxStyle(box)}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        lastY.current = e.clientY;
      }}
      onPointerMove={(e) => {
        if (lastY.current != null && e.currentTarget.hasPointerCapture(e.pointerId)) {
          session.addScroll(0, (e.clientY - lastY.current) * 1.5);
          lastY.current = e.clientY;
        }
      }}
      onPointerUp={() => (lastY.current = null)}
      onPointerCancel={() => (lastY.current = null)}
    >
      <div className="rail" />
    </div>
  );
}

// ---- trigger ----

function TriggerControl({
  control,
  box,
  session,
}: {
  control: Control;
  box: Box;
  session: ControllerSession;
}) {
  const [value, setValue] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  const index = control.id === "r2" ? 1 : 0;

  const apply = (v: number) => {
    const clamped = Math.max(0, Math.min(1, v));
    setValue(clamped);
    const triggers: [number, number] = [...session.triggers];
    triggers[index] = clamped;
    session.triggers = triggers;
  };

  const fromPoint = (clientY: number) => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    apply(1 - (clientY - rect.top) / rect.height);
  };

  return (
    <div
      ref={ref}
      className="ctl ctl-trigger control-surface"
      style={boxStyle(box)}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        fromPoint(e.clientY);
      }}
      onPointerMove={(e) => {
        if (e.currentTarget.hasPointerCapture(e.pointerId)) fromPoint(e.clientY);
      }}
      onPointerUp={() => apply(0)}
      onPointerCancel={() => apply(0)}
    >
      <div className="fill" style={{ height: `${value * 100}%` }} />
      <span className="lbl">{control.label ?? control.id}</span>
    </div>
  );
}

// ---- phone keyboard ----

const CHAR_CODES: Record<string, string> = {
  " ": "Space",
  "-": "Minus",
  "=": "Equal",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  ";": "Semicolon",
  "'": "Quote",
  "`": "Backquote",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
};

function charToCode(ch: string): string | null {
  if (/^[a-z]$/i.test(ch)) return `Key${ch.toUpperCase()}`;
  if (/^[0-9]$/.test(ch)) return `Digit${ch}`;
  return CHAR_CODES[ch] ?? null;
}

function KeyboardControl({ box, session }: { box: Box; session: ControllerSession }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [active, setActive] = useState(false);

  return (
    <div className="ctl ctl-keyboard" style={boxStyle(box)}>
      <button
        style={{ width: "100%", height: "100%", color: "inherit" }}
        onClick={() => {
          inputRef.current?.focus();
        }}
      >
        {active ? "typing... (tap elsewhere to stop)" : "tap to type on the computer"}
      </button>
      <input
        ref={inputRef}
        aria-label="Remote keyboard input"
        style={{
          position: "absolute",
          opacity: 0,
          pointerEvents: "none",
          width: 1,
          height: 1,
        }}
        autoCapitalize="none"
        autoCorrect="off"
        autoComplete="off"
        onFocus={() => setActive(true)}
        onBlur={() => setActive(false)}
        onKeyDown={(e) => {
          // Hardware-style events carry a usable code; soft keyboards may
          // report Unidentified and are handled in onBeforeInput instead.
          if (e.code && e.code !== "Unidentified") {
            e.preventDefault();
            session.sendKey(e.code, true);
          }
        }}
        onKeyUp={(e) => {
          if (e.code && e.code !== "Unidentified") {
            e.preventDefault();
            session.sendKey(e.code, false);
          }
        }}
        onBeforeInput={(e) => {
          const event = e.nativeEvent as InputEvent;
          e.preventDefault();
          if (event.inputType === "insertText" && event.data) {
            for (const ch of event.data) {
              const code = charToCode(ch);
              if (code) session.tapKey(code);
            }
          } else if (event.inputType === "deleteContentBackward") {
            session.tapKey("Backspace");
          } else if (
            event.inputType === "insertLineBreak" ||
            event.inputType === "insertParagraph"
          ) {
            session.tapKey("Enter");
          }
        }}
      />
    </div>
  );
}

// ---- motion block ----

function MotionControl({
  control,
  box,
  on,
  session,
}: {
  control: Control;
  box: Box;
  on: boolean;
  session: ControllerSession;
}) {
  return (
    <div className={`ctl ctl-motion ${on ? "on" : ""}`} style={boxStyle(box)}>
      <span>{control.label ?? "Motion"}</span>
      <span>{on ? "streaming" : "off"}</span>
      {on && (
        <button className="btn" onClick={() => session.recenter()}>
          Recenter
        </button>
      )}
    </div>
  );
}
