use std::sync::OnceLock;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn frequency() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut f);
        }
        if f == 0 {
            10_000_000
        } else {
            f
        }
    })
}

fn counter() -> i64 {
    let mut c = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
    }
    c
}

#[derive(Clone, Copy)]
pub struct Instant {
    ticks: i64,
}

impl Instant {
    pub fn now() -> Self {
        Self { ticks: counter() }
    }

    pub fn after(interval: i64) -> Self {
        let step = interval.saturating_mul(frequency()) / 10_000_000;
        Self { ticks: counter() + step }
    }

    pub fn elapsed_ms(&self) -> f64 {
        (counter() - self.ticks) as f64 * 1000.0 / frequency() as f64
    }

    pub fn until_100ns(&self) -> i64 {
        (self.ticks - counter()).saturating_mul(10_000_000) / frequency()
    }

    pub fn advance_100ns(&mut self, interval: i64) {
        let step = interval.saturating_mul(frequency()) / 10_000_000;
        self.ticks += step;

        let now = counter();
        if self.ticks < now - step * 3 {
            self.ticks = now + step;
        }
    }
}

fn filetime_to_100ns(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

pub struct Monitor {
    enabled: bool,
    frames: u64,
    last_report: Instant,
    last_cpu_100ns: u64,
}

impl Monitor {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            frames: 0,
            last_report: Instant::now(),
            last_cpu_100ns: process_cpu_100ns(),
        }
    }

    pub fn tick(&mut self) {
        if !self.enabled {
            return;
        }

        self.frames += 1;

        let elapsed_ms = self.last_report.elapsed_ms();
        if elapsed_ms < 5000.0 {
            return;
        }

        let cpu_now = process_cpu_100ns();
        let cpu_delta_ms = (cpu_now - self.last_cpu_100ns) as f64 / 10_000.0;
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;

        let (working_set_kb, private_kb) = process_memory_kb();

        println!(
            "{:>6.2} fps | cpu {:>5.2}% of one core ({:>5.3}% of the system) | working set {} KB | private {} KB",
            self.frames as f64 * 1000.0 / elapsed_ms,
            cpu_delta_ms * 100.0 / elapsed_ms,
            cpu_delta_ms * 100.0 / elapsed_ms / cores,
            working_set_kb,
            private_kb
        );

        self.frames = 0;
        self.last_report = Instant::now();
        self.last_cpu_100ns = cpu_now;
    }
}

fn process_cpu_100ns() -> u64 {
    let (mut creation, mut exit, mut kernel, mut user) = Default::default();
    unsafe {
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .is_err()
        {
            return 0;
        }
    }
    filetime_to_100ns(kernel) + filetime_to_100ns(user)
}

fn process_memory_kb() -> (u64, u64) {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };

    unsafe {
        if GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .is_err()
        {
            return (0, 0);
        }
    }

    (
        counters.WorkingSetSize as u64 / 1024,
        counters.PagefileUsage as u64 / 1024,
    )
}

pub struct Phases {
    enabled: bool,
    start: Instant,
    last_ms: f64,
}

impl Phases {
    pub fn new(enabled: bool, start: Instant) -> Self {
        Self { enabled, start, last_ms: 0.0 }
    }

    pub fn mark(&mut self, label: &str) {
        if !self.enabled {
            return;
        }
        let now = self.start.elapsed_ms();
        println!("  {:>8.1} ms  +{:>7.1} ms  {}", now, now - self.last_ms, label);
        self.last_ms = now;
    }
}

pub fn local_minute_of_day() -> u16 {
    let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    time.wHour * 60 + time.wMinute
}
