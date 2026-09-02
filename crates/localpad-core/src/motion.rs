//! Gyroscope processing: normalization, calibration offset, low-pass
//! smoothing, dead zones and sensitivity, plus the mappings from motion to
//! mouse movement, right stick or steering.

use serde::{Deserialize, Serialize};

/// A calibrated motion sample ready for an output adapter (DSU wants raw
/// calibrated motion; other outputs derive axes from it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct MotionSample {
    /// Orientation relative to the recenter pose, unit quaternion [x, y, z, w].
    pub orientation: [f32; 4],
    /// Smoothed angular velocity in degrees per second: [pitch, yaw, roll].
    pub angular_velocity: [f32; 3],
    /// Acceleration including gravity, in g.
    pub acceleration: [f32; 3],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionSettings {
    /// Pointer pixels per degree of rotation for gyro-to-mouse.
    pub mouse_sensitivity: f32,
    /// Degrees of tilt that map to a full stick deflection.
    pub stick_range_degrees: f32,
    /// Angular velocity below this many degrees per second is ignored.
    pub dead_zone_dps: f32,
    /// Exponential smoothing factor 0..1; higher follows the raw signal
    /// more closely, lower smooths harder.
    pub smoothing: f32,
    /// Response curve exponent; 1.0 is linear, 2.0 emphasizes fine motion.
    pub curve: f32,
}

impl Default for MotionSettings {
    fn default() -> Self {
        MotionSettings {
            mouse_sensitivity: 8.0,
            stick_range_degrees: 35.0,
            dead_zone_dps: 1.5,
            smoothing: 0.5,
            curve: 1.0,
        }
    }
}

fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

fn quat_conjugate(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

fn quat_multiply(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// Extract intrinsic pitch (x), yaw (y) and roll (z) in degrees from a
/// relative orientation quaternion.
pub fn quat_to_euler_degrees(q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let sinp = 2.0 * (w * x - y * z);
    let pitch = if sinp.abs() >= 1.0 {
        90.0_f32.copysign(sinp)
    } else {
        sinp.asin().to_degrees()
    };
    let yaw = (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (x * x + y * y)).to_degrees();
    let roll = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (x * x + z * z)).to_degrees();
    [pitch, yaw, roll]
}

#[derive(Debug, Default)]
pub struct MotionProcessor {
    pub settings: MotionSettings,
    /// Inverse of the orientation captured at the last recenter.
    reference_inverse: Option<[f32; 4]>,
    last_orientation: Option<[f32; 4]>,
    smoothed_velocity: [f32; 3],
    pending_recenter: bool,
}

impl MotionProcessor {
    pub fn new(settings: MotionSettings) -> Self {
        MotionProcessor {
            settings,
            ..Default::default()
        }
    }

    /// Ask for the next processed orientation to become the neutral pose.
    pub fn recenter(&mut self) {
        self.pending_recenter = true;
    }

    pub fn reset(&mut self) {
        self.reference_inverse = None;
        self.last_orientation = None;
        self.smoothed_velocity = [0.0; 3];
        self.pending_recenter = false;
    }

    /// Process one sanitized frame's motion data into a calibrated sample.
    /// Returns None when the frame carries no orientation.
    pub fn process(
        &mut self,
        orientation: Option<[f32; 4]>,
        angular_velocity: Option<[f32; 3]>,
        acceleration: Option<[f32; 3]>,
    ) -> Option<MotionSample> {
        let raw = quat_normalize(orientation?);
        self.last_orientation = Some(raw);
        if self.pending_recenter || self.reference_inverse.is_none() {
            self.reference_inverse = Some(quat_conjugate(raw));
            self.pending_recenter = false;
        }
        let relative = quat_normalize(quat_multiply(self.reference_inverse.unwrap(), raw));

        let alpha = self.settings.smoothing.clamp(0.05, 1.0);
        let velocity = angular_velocity.unwrap_or([0.0; 3]);
        for (smoothed, raw) in self.smoothed_velocity.iter_mut().zip(velocity) {
            let v = if raw.abs() < self.settings.dead_zone_dps { 0.0 } else { raw };
            *smoothed += alpha * (v - *smoothed);
        }

        Some(MotionSample {
            orientation: relative,
            angular_velocity: self.smoothed_velocity,
            acceleration: acceleration.unwrap_or([0.0, 0.0, 1.0]),
        })
    }

    fn apply_curve(&self, value: f32) -> f32 {
        let curve = self.settings.curve.clamp(0.5, 3.0);
        value.signum() * value.abs().powf(curve)
    }

    /// Map smoothed angular velocity to a pointer delta for one frame.
    /// `dt` is the frame interval in seconds.
    pub fn to_mouse_delta(&self, sample: &MotionSample, dt: f32) -> [f32; 2] {
        let sens = self.settings.mouse_sensitivity;
        let dx = -sample.angular_velocity[1] * dt * sens;
        let dy = -sample.angular_velocity[0] * dt * sens;
        [self.apply_curve(dx / 10.0) * 10.0, self.apply_curve(dy / 10.0) * 10.0]
    }

    /// Map relative tilt to a stick position.
    pub fn to_stick(&self, sample: &MotionSample) -> [f32; 2] {
        let [pitch, yaw, _] = quat_to_euler_degrees(sample.orientation);
        let range = self.settings.stick_range_degrees.max(5.0);
        [(-yaw / range).clamp(-1.0, 1.0), (-pitch / range).clamp(-1.0, 1.0)]
    }

    /// Map roll to a single steering axis.
    pub fn to_steering(&self, sample: &MotionSample) -> f32 {
        let [_, _, roll] = quat_to_euler_degrees(sample.orientation);
        let range = self.settings.stick_range_degrees.max(5.0);
        (roll / range).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quat_from_yaw_degrees(deg: f32) -> [f32; 4] {
        let half = deg.to_radians() / 2.0;
        [0.0, half.sin(), 0.0, half.cos()]
    }

    #[test]
    fn recenter_zeroes_orientation() {
        let mut p = MotionProcessor::default();
        let q = quat_from_yaw_degrees(40.0);
        let s = p.process(Some(q), None, None).unwrap();
        // First sample becomes the reference, so relative orientation is identity.
        let [pitch, yaw, roll] = quat_to_euler_degrees(s.orientation);
        assert!(pitch.abs() < 0.01 && yaw.abs() < 0.01 && roll.abs() < 0.01);
    }

    #[test]
    fn relative_yaw_tracks_reference() {
        let mut p = MotionProcessor::default();
        p.process(Some(quat_from_yaw_degrees(10.0)), None, None).unwrap();
        let s = p.process(Some(quat_from_yaw_degrees(30.0)), None, None).unwrap();
        let [_, yaw, _] = quat_to_euler_degrees(s.orientation);
        assert!((yaw - 20.0).abs() < 0.5, "yaw was {yaw}");
        // Recenter and the same pose reads as zero again.
        p.recenter();
        let s = p.process(Some(quat_from_yaw_degrees(30.0)), None, None).unwrap();
        let [_, yaw, _] = quat_to_euler_degrees(s.orientation);
        assert!(yaw.abs() < 0.01, "yaw after recenter was {yaw}");
    }

    #[test]
    fn dead_zone_swallows_small_motion() {
        let mut p = MotionProcessor::new(MotionSettings {
            dead_zone_dps: 2.0,
            smoothing: 1.0,
            ..Default::default()
        });
        let s = p
            .process(Some([0.0, 0.0, 0.0, 1.0]), Some([1.0, -1.5, 0.5]), None)
            .unwrap();
        assert_eq!(s.angular_velocity, [0.0, 0.0, 0.0]);
        let s = p
            .process(Some([0.0, 0.0, 0.0, 1.0]), Some([10.0, 0.0, 0.0]), None)
            .unwrap();
        assert_eq!(s.angular_velocity, [10.0, 0.0, 0.0]);
    }

    #[test]
    fn stick_mapping_clamps() {
        let p = MotionProcessor::default();
        let sample = MotionSample {
            orientation: quat_from_yaw_degrees(90.0),
            ..Default::default()
        };
        let [x, _] = p.to_stick(&sample);
        assert!((-1.0..=1.0).contains(&x));
        assert!((x - -1.0).abs() < 0.01 || (x - 1.0).abs() < 0.01);
    }
}
