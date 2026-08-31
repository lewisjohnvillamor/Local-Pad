// Device motion capture: permission flow, orientation quaternion with
// screen-angle correction, angular velocity and acceleration in the units
// the server expects (degrees per second, g).

export type Quat = [number, number, number, number];

const DEG = Math.PI / 180;

export function quatFromEuler(alpha: number, beta: number, gamma: number): Quat {
  // W3C device orientation: intrinsic Tait-Bryan angles Z (alpha),
  // X (beta), Y (gamma).
  const x = (beta * DEG) / 2;
  const y = (gamma * DEG) / 2;
  const z = (alpha * DEG) / 2;
  const cX = Math.cos(x);
  const cY = Math.cos(y);
  const cZ = Math.cos(z);
  const sX = Math.sin(x);
  const sY = Math.sin(y);
  const sZ = Math.sin(z);
  return [
    sX * cY * cZ - cX * sY * sZ,
    cX * sY * cZ + sX * cY * sZ,
    cX * cY * sZ + sX * sY * cZ,
    cX * cY * cZ - sX * sY * sZ,
  ];
}

function quatMultiply(a: Quat, b: Quat): Quat {
  const [ax, ay, az, aw] = a;
  const [bx, by, bz, bw] = b;
  return [
    aw * bx + ax * bw + ay * bz - az * by,
    aw * by - ax * bz + ay * bw + az * bx,
    aw * bz + ax * by - ay * bx + az * bw,
    aw * bw - ax * bx - ay * by - az * bz,
  ];
}

function screenAngle(): number {
  if (screen.orientation && typeof screen.orientation.angle === "number") {
    return screen.orientation.angle;
  }
  return 0;
}

/// Rotate the device quaternion so axes follow the screen, not the body.
function adjustForScreen(q: Quat): Quat {
  const angle = (-screenAngle() * DEG) / 2;
  const rotation: Quat = [0, 0, Math.sin(angle), Math.cos(angle)];
  return quatMultiply(q, rotation);
}

export interface MotionSample {
  orientation: Quat | null;
  angularVelocity: [number, number, number] | null;
  acceleration: [number, number, number] | null;
}

type PermissionRequester = { requestPermission?: () => Promise<"granted" | "denied"> };

export async function requestMotionPermission(): Promise<boolean> {
  const OrientationEvent = DeviceOrientationEvent as typeof DeviceOrientationEvent &
    PermissionRequester;
  const MotionEvent = DeviceMotionEvent as typeof DeviceMotionEvent & PermissionRequester;
  try {
    if (typeof OrientationEvent.requestPermission === "function") {
      const granted = (await OrientationEvent.requestPermission()) === "granted";
      if (!granted) return false;
    }
    if (typeof MotionEvent.requestPermission === "function") {
      return (await MotionEvent.requestPermission()) === "granted";
    }
    return "DeviceOrientationEvent" in window;
  } catch {
    return false;
  }
}

export class MotionCapture {
  private latest: MotionSample = {
    orientation: null,
    angularVelocity: null,
    acceleration: null,
  };
  private orientationHandler = (event: DeviceOrientationEvent) => {
    if (event.alpha == null || event.beta == null || event.gamma == null) return;
    this.latest.orientation = adjustForScreen(
      quatFromEuler(event.alpha, event.beta, event.gamma)
    );
  };
  private motionHandler = (event: DeviceMotionEvent) => {
    const rate = event.rotationRate;
    if (rate && rate.alpha != null && rate.beta != null && rate.gamma != null) {
      // [pitch, yaw, roll] in degrees per second.
      this.latest.angularVelocity = [rate.beta, rate.alpha, rate.gamma];
    }
    const accel = event.accelerationIncludingGravity;
    if (accel && accel.x != null && accel.y != null && accel.z != null) {
      this.latest.acceleration = [accel.x / 9.81, accel.y / 9.81, accel.z / 9.81];
    }
  };
  private active = false;

  start(): void {
    if (this.active) return;
    this.active = true;
    window.addEventListener("deviceorientation", this.orientationHandler);
    window.addEventListener("devicemotion", this.motionHandler);
  }

  stop(): void {
    if (!this.active) return;
    this.active = false;
    window.removeEventListener("deviceorientation", this.orientationHandler);
    window.removeEventListener("devicemotion", this.motionHandler);
    this.latest = { orientation: null, angularVelocity: null, acceleration: null };
  }

  get isActive(): boolean {
    return this.active;
  }

  sample(): MotionSample {
    return this.latest;
  }
}
