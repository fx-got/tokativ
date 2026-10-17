//! How much memory and processor time the daemon takes, read from `/proc` for the settings
//! window.

use std::fs;
use std::time::Instant;

/// One reading of the daemon's footprint.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    /// Resident memory in bytes. Mostly the loaded models.
    pub memory: u64,
    /// Processor time since the previous reading, as a percentage of one core. `None` on the
    /// first reading, which has nothing to compare against.
    pub cpu: Option<f32>,
}

/// Takes readings of this process. CPU use needs two readings, so it keeps the last one.
#[derive(Default)]
pub struct Sampler {
    last: Option<(Instant, u64)>,
}

impl Sampler {
    pub fn sample(&mut self) -> Usage {
        let now = Instant::now();
        let ticks = cpu_ticks();
        let cpu = match (self.last, ticks) {
            (Some((then, before)), Some(ticks)) => {
                let seconds = now.duration_since(then).as_secs_f32();
                let used = ticks.saturating_sub(before) as f32
                    / rustix::param::clock_ticks_per_second() as f32;
                (seconds > 0.).then(|| used / seconds * 100.)
            }
            _ => None,
        };
        if let Some(ticks) = ticks {
            self.last = Some((now, ticks));
        }
        Usage {
            memory: resident().unwrap_or(0),
            cpu,
        }
    }
}

/// User and system time this process has used, in clock ticks.
fn cpu_ticks() -> Option<u64> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    // The command name may hold spaces, so fields are counted from after its closing paren.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let user: u64 = fields.get(11)?.parse().ok()?;
    let system: u64 = fields.get(12)?.parse().ok()?;
    Some(user + system)
}

/// Resident memory in bytes.
fn resident() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kilobytes * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_come_from_proc() {
        let mut sampler = Sampler::default();
        let first = sampler.sample();
        assert!(first.memory > 0);
        assert!(first.cpu.is_none());
        let busy = Instant::now();
        while busy.elapsed().as_millis() < 50 {
            std::hint::black_box(busy.elapsed());
        }
        assert!(sampler.sample().cpu.is_some());
    }
}
