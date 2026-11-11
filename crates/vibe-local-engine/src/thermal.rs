//! Thermal governor: real sensor reads where the OS exposes them, with
//! 42°C throttle / 45°C pause thresholds and hysteresis. When no sensor is
//! available the state is `NoSensor` — temperatures are never invented.
//! Android can inject readings from the Kotlin ThermalGovernor via
//! [`ThermalGovernor::set_override`].

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use vibe_local_core::util::run_with_timeout;

pub const THROTTLE_C: f64 = 42.0;
pub const PAUSE_C: f64 = 45.0;
pub const RESUME_C: f64 = 43.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThermalState {
    Normal,
    Throttled,
    Paused,
    NoSensor,
}

/// Read the hottest relevant CPU/SoC thermal zone in °C, if available.
pub fn read_celsius() -> Option<f64> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        linux_thermal_c()
    }
    #[cfg(target_os = "windows")]
    {
        windows_thermal_c()
    }
    #[cfg(target_os = "macos")]
    {
        macos_thermal_c()
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "windows",
        target_os = "macos"
    )))]
    {
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn linux_thermal_c() -> Option<f64> {
    let mut best: Option<f64> = None;
    let entries = std::fs::read_dir("/sys/class/thermal").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("thermal_zone") {
            continue;
        }
        let kind = std::fs::read_to_string(entry.path().join("type")).unwrap_or_default();
        let kind = kind.trim().to_lowercase();
        // Prefer CPU/SoC/package zones; skip battery/charger sensors.
        let relevant = ["cpu", "x86", "pkg", "soc", "tsens", "core", "acpi"]
            .iter()
            .any(|k| kind.contains(k));
        let excluded = ["battery", "charger", "skin"]
            .iter()
            .any(|k| kind.contains(k));
        if excluded || (!relevant && !kind.is_empty()) {
            continue;
        }
        let raw = std::fs::read_to_string(entry.path().join("temp")).ok()?;
        let Ok(milli) = raw.trim().parse::<f64>() else {
            continue;
        };
        if milli <= 0.0 || milli > 150_000.0 {
            continue;
        }
        let c = milli / 1000.0;
        best = Some(best.map_or(c, |b: f64| b.max(c)));
    }
    best
}

#[cfg(target_os = "windows")]
fn windows_thermal_c() -> Option<f64> {
    // ACPI thermal zones via WMI (tenths of Kelvin). Many desktops do not
    // expose this class; in that case we honestly report no sensor.
    let out = run_with_timeout(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance -Namespace root/wmi -ClassName MSAcpi_ThermalZoneTemperature -ErrorAction SilentlyContinue | Select-Object -First 1).CurrentTemperature",
        ],
        Duration::from_secs(8),
    )?;
    let tenth_kelvin: f64 = out.trim().parse().ok()?;
    if tenth_kelvin <= 0.0 {
        return None;
    }
    Some((tenth_kelvin - 2732.0) / 10.0)
}

#[cfg(target_os = "macos")]
fn macos_thermal_c() -> Option<f64> {
    // `osx-cpu-temp` is the common open-source SMC reader; without it macOS
    // exposes no public CPU temperature API.
    let out = run_with_timeout("osx-cpu-temp", &[], Duration::from_secs(5))?;
    out.trim()
        .trim_end_matches('°')
        .trim_end_matches('C')
        .trim()
        .parse::<f64>()
        .ok()
}

pub struct ThermalGovernor {
    state: std::sync::Mutex<ThermalState>,
    /// Millidegrees override injected by Android (i64::MIN = none).
    override_milli: AtomicI64,
}

impl Default for ThermalGovernor {
    fn default() -> Self {
        Self::new()
    }
}

impl ThermalGovernor {
    pub fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(ThermalState::NoSensor),
            override_milli: AtomicI64::new(i64::MIN),
        }
    }

    /// Inject a device temperature reading (Android ThermalGovernor bridge).
    pub fn set_override(&self, celsius: f64) {
        self.override_milli
            .store((celsius * 1000.0) as i64, Ordering::Relaxed);
    }

    pub fn clear_override(&self) {
        self.override_milli.store(i64::MIN, Ordering::Relaxed);
    }

    pub fn current_reading(&self) -> Option<f64> {
        let ov = self.override_milli.load(Ordering::Relaxed);
        if ov != i64::MIN {
            return Some(ov as f64 / 1000.0);
        }
        read_celsius()
    }

    /// Re-evaluate state from the current reading; returns the new state.
    pub fn poll(&self) -> ThermalState {
        let reading = self.current_reading();
        let mut state = self.state.lock().unwrap();
        *state = match reading {
            None => ThermalState::NoSensor,
            Some(c) => match *state {
                ThermalState::Paused => {
                    if c < RESUME_C {
                        ThermalState::Normal
                    } else {
                        ThermalState::Paused
                    }
                }
                _ => {
                    if c >= PAUSE_C {
                        ThermalState::Paused
                    } else if c >= THROTTLE_C {
                        ThermalState::Throttled
                    } else {
                        ThermalState::Normal
                    }
                }
            },
        };
        *state
    }

    pub fn state(&self) -> ThermalState {
        *self.state.lock().unwrap()
    }

    /// Extra inter-token delay (ms) imposed by the governor.
    pub fn throttle_delay_ms(&self) -> u64 {
        match self.state() {
            ThermalState::Throttled => 25,
            ThermalState::Paused => 500,
            _ => 0,
        }
    }

    /// Whether new inference requests may start.
    pub fn allows_inference(&self) -> bool {
        !matches!(self.state(), ThermalState::Paused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governor_transitions() {
        let g = ThermalGovernor::new();
        g.set_override(35.0);
        assert_eq!(g.poll(), ThermalState::Normal);
        assert!(g.allows_inference());
        g.set_override(43.0);
        assert_eq!(g.poll(), ThermalState::Throttled);
        assert_eq!(g.throttle_delay_ms(), 25);
        g.set_override(46.0);
        assert_eq!(g.poll(), ThermalState::Paused);
        assert!(!g.allows_inference());
        // Hysteresis: 44°C stays paused until below 43.
        g.set_override(44.0);
        assert_eq!(g.poll(), ThermalState::Paused);
        g.set_override(42.5);
        assert_eq!(g.poll(), ThermalState::Normal);
    }

    #[test]
    fn no_sensor_is_honest() {
        let g = ThermalGovernor::new();
        g.clear_override();
        // On machines without sensors this is NoSensor; with sensors Normal.
        let state = g.poll();
        assert!(matches!(
            state,
            ThermalState::NoSensor | ThermalState::Normal
        ));
        assert!(g.allows_inference());
    }

    #[test]
    fn override_roundtrip() {
        let g = ThermalGovernor::new();
        g.set_override(41.5);
        assert_eq!(g.current_reading(), Some(41.5));
        g.clear_override();
    }
}
