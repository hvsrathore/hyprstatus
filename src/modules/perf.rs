use crate::config::{parse_hex_color, Config};
use crate::modules::{ClickAction, ClickRegion, ModuleBlock};
use crate::renderer::SoftwareRenderer;
use std::fs;
use std::sync::{Arc, RwLock};

/// Fixed-size ring buffer for rolling time-series data
#[derive(Debug, Clone)]
pub struct RollingRingBuffer<const N: usize> {
    data: [f32; N],
    head: usize,
    count: usize,
}

impl<const N: usize> RollingRingBuffer<N> {
    pub const fn new() -> Self {
        Self {
            data: [0.0; N],
            head: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, val: f32) {
        self.data[self.head] = val;
        self.head = (self.head + 1) % N;
        if self.count < N {
            self.count += 1;
        }
    }

    pub fn to_vec(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.count);
        if self.count < N {
            for i in 0..self.count {
                out.push(self.data[i]);
            }
        } else {
            for i in 0..N {
                let idx = (self.head + i) % N;
                out.push(self.data[idx]);
            }
        }
        out
    }
}

pub struct PerfModule {
    cpu_history: Arc<RwLock<RollingRingBuffer<16>>>,
    mem_history: Arc<RwLock<RollingRingBuffer<16>>>,
    last_cpu_percent: Arc<RwLock<f32>>,
    last_mem_percent: Arc<RwLock<f32>>,
}

impl PerfModule {
    pub fn new() -> Self {
        let cpu_history = Arc::new(RwLock::new(RollingRingBuffer::new()));
        let mem_history = Arc::new(RwLock::new(RollingRingBuffer::new()));
        let last_cpu_percent = Arc::new(RwLock::new(0.0));
        let last_mem_percent = Arc::new(RwLock::new(0.0));

        let module = Self {
            cpu_history: cpu_history.clone(),
            mem_history: mem_history.clone(),
            last_cpu_percent: last_cpu_percent.clone(),
            last_mem_percent: last_mem_percent.clone(),
        };

        module.spawn_sampler(cpu_history, mem_history, last_cpu_percent, last_mem_percent);
        module
    }

    fn spawn_sampler(
        &self,
        cpu_hist: Arc<RwLock<RollingRingBuffer<16>>>,
        mem_hist: Arc<RwLock<RollingRingBuffer<16>>>,
        last_cpu: Arc<RwLock<f32>>,
        last_mem: Arc<RwLock<f32>>,
    ) {
        tokio::spawn(async move {
            let mut prev_total = 0u64;
            let mut prev_idle = 0u64;

            loop {
                // Sample CPU from /proc/stat
                if let Ok(stat) = fs::read_to_string("/proc/stat") {
                    if let Some(first_line) = stat.lines().next() {
                        let parts: Vec<&str> = first_line.split_whitespace().collect();
                        if parts.len() >= 5 && parts[0] == "cpu" {
                            let user: u64 = parts[1].parse().unwrap_or(0);
                            let nice: u64 = parts[2].parse().unwrap_or(0);
                            let system: u64 = parts[3].parse().unwrap_or(0);
                            let idle: u64 = parts[4].parse().unwrap_or(0);
                            let iowait: u64 = parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0);
                            let irq: u64 = parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
                            let softirq: u64 = parts.get(7).and_then(|v| v.parse().ok()).unwrap_or(0);
                            let steal: u64 = parts.get(8).and_then(|v| v.parse().ok()).unwrap_or(0);

                            let total = user + nice + system + idle + iowait + irq + softirq + steal;
                            let idle_sum = idle + iowait;

                            if prev_total > 0 && total > prev_total {
                                let d_total = (total - prev_total) as f32;
                                let d_idle = (idle_sum.saturating_sub(prev_idle)) as f32;
                                let usage = ((d_total - d_idle) / d_total).clamp(0.0, 1.0);

                                {
                                    let mut hist = cpu_hist.write().unwrap();
                                    hist.push(usage);
                                }
                                {
                                    let mut val = last_cpu.write().unwrap();
                                    *val = usage * 100.0;
                                }
                            }

                            prev_total = total;
                            prev_idle = idle_sum;
                        }
                    }
                }

                // Sample Memory from /proc/meminfo
                if let Ok(mem) = fs::read_to_string("/proc/meminfo") {
                    let mut mem_total = 0u64;
                    let mut mem_avail = 0u64;

                    for line in mem.lines() {
                        if line.starts_with("MemTotal:") {
                            if let Some(num_str) = line.split_whitespace().nth(1) {
                                mem_total = num_str.parse().unwrap_or(0);
                            }
                        } else if line.starts_with("MemAvailable:") {
                            if let Some(num_str) = line.split_whitespace().nth(1) {
                                mem_avail = num_str.parse().unwrap_or(0);
                            }
                        }
                    }

                    if mem_total > 0 {
                        let used = mem_total.saturating_sub(mem_avail);
                        let usage = (used as f32 / mem_total as f32).clamp(0.0, 1.0);

                        {
                            let mut hist = mem_hist.write().unwrap();
                            hist.push(usage);
                        }
                        {
                            let mut val = last_mem.write().unwrap();
                            *val = usage * 100.0;
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
            }
        });
    }

    pub fn render(&self, config: &Config) -> ModuleBlock {
        let cpu_vec = self.cpu_history.read().map(|h| h.to_vec()).unwrap_or_default();
        let mem_vec = self.mem_history.read().map(|h| h.to_vec()).unwrap_or_default();

        let cpu_pct = self.last_cpu_percent.read().map(|v| *v).unwrap_or(0.0);
        let mem_pct = self.last_mem_percent.read().map(|v| *v).unwrap_or(0.0);

        let cpu_sparkline = SoftwareRenderer::render_braille_sparkline(&cpu_vec, 1.0);
        let mem_sparkline = SoftwareRenderer::render_braille_sparkline(&mem_vec, 1.0);

        let text = format!(" {} {:.0}%   󰍛 {} {:.0}%", cpu_sparkline, cpu_pct, mem_sparkline, mem_pct);

        let fg = parse_hex_color(&config.colors.cpu);
        let bg = parse_hex_color(&config.colors.block_background);

        let mut block = ModuleBlock::new("perf", &text, fg, bg);
        block.icon = "".to_string();
        block.sparkline = Some(cpu_sparkline);

        block.click_regions.push(ClickRegion {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            action: ClickAction::PerfClick,
        });

        block
    }
}
