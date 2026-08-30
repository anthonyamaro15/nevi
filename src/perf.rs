use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::time::Duration;

const MAX_SAMPLES_PER_METRIC: usize = 10_000;
const DEFAULT_FLIGHT_RECORDER_CAPACITY: usize = 2_048;

#[derive(Debug, Default)]
pub struct PerfStats {
    metrics: BTreeMap<String, MetricStats>,
}

#[derive(Debug, Default)]
struct MetricStats {
    count: u64,
    total_us: u128,
    max_us: u128,
    samples_us: Vec<u128>,
}

impl PerfStats {
    pub fn record(&mut self, name: impl Into<String>, duration: Duration) {
        let elapsed_us = duration.as_micros();
        let metric = self.metrics.entry(name.into()).or_default();
        metric.count += 1;
        metric.total_us += elapsed_us;
        metric.max_us = metric.max_us.max(elapsed_us);
        if metric.samples_us.len() < MAX_SAMPLES_PER_METRIC {
            metric.samples_us.push(elapsed_us);
        }
    }

    pub fn summary_lines(&self) -> Vec<String> {
        self.metrics
            .iter()
            .map(|(name, metric)| {
                let avg_us = metric.total_us / u128::from(metric.count);
                let mut samples = metric.samples_us.clone();
                samples.sort_unstable();
                let p50_us = percentile(&samples, 50);
                let p95_us = percentile(&samples, 95);

                format!(
                    "{name} count={} samples={} total_us={} avg_us={} p50_us={} p95_us={} max_us={}",
                    metric.count,
                    metric.samples_us.len(),
                    metric.total_us,
                    avg_us,
                    p50_us,
                    p95_us,
                    metric.max_us
                )
            })
            .collect()
    }

    pub fn write_summary(&self, mut writer: impl Write) -> io::Result<()> {
        if self.metrics.is_empty() {
            return Ok(());
        }

        writeln!(writer, "# profile summary")?;
        for line in self.summary_lines() {
            writeln!(writer, "{line}")?;
        }
        Ok(())
    }
}

fn percentile(sorted_samples: &[u128], percentile: u32) -> u128 {
    if sorted_samples.is_empty() {
        return 0;
    }

    let percentile = percentile.min(100) as usize;
    let rank = (percentile * sorted_samples.len()).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted_samples.len() - 1);
    sorted_samples[index]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlightEvent {
    pub sequence: u64,
    pub name: &'static str,
    pub duration_us: u128,
    pub slow: bool,
}

#[derive(Debug)]
pub struct FlightRecorder {
    capacity: usize,
    next_sequence: u64,
    events: VecDeque<FlightEvent>,
}

impl Default for FlightRecorder {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_FLIGHT_RECORDER_CAPACITY)
    }
}

impl FlightRecorder {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_sequence: 0,
            events: VecDeque::new(),
        }
    }

    pub fn record(&mut self, name: &'static str, duration: Duration) {
        let duration_us = duration.as_micros();
        let slow = duration_us >= slow_threshold_us(name);
        let event = FlightEvent {
            sequence: self.next_sequence,
            name,
            duration_us,
            slow,
        };
        self.next_sequence = self.next_sequence.wrapping_add(1);

        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    pub fn render_report(&self) -> String {
        let mut report = String::new();
        report.push_str("# Nevi Flight Recorder\n\n");
        report.push_str("## Overview\n");
        report.push_str(&format!(
            "- Events retained: {} / {}\n",
            self.events.len(),
            self.capacity
        ));
        report.push_str("- Storage: in-memory ring buffer only\n");
        report.push_str("- Verbose file logging remains opt-in with `NEVI_PROFILE=1`\n\n");

        if self.events.is_empty() {
            report.push_str("No timing events recorded yet.\n");
            return report;
        }

        report.push_str("## Summary\n");
        for line in self.summary_lines() {
            report.push_str(&format!("- {line}\n"));
        }
        report.push('\n');

        let slow_events: Vec<&FlightEvent> =
            self.events.iter().filter(|event| event.slow).collect();
        report.push_str("## Slow Events\n");
        if slow_events.is_empty() {
            report.push_str("- none\n\n");
        } else {
            for event in slow_events.iter().rev().take(20).rev() {
                report.push_str(&format!(
                    "- #{} {} {} slow\n",
                    event.sequence,
                    event.name,
                    format_duration_us(event.duration_us)
                ));
            }
            report.push('\n');
        }

        report.push_str("## Recent Events\n");
        for event in self.events.iter().rev().take(40).rev() {
            let marker = if event.slow { " slow" } else { "" };
            report.push_str(&format!(
                "- #{} {} {}{}\n",
                event.sequence,
                event.name,
                format_duration_us(event.duration_us),
                marker
            ));
        }

        report
    }

    fn summary_lines(&self) -> Vec<String> {
        let mut metrics: BTreeMap<&'static str, Vec<u128>> = BTreeMap::new();
        for event in &self.events {
            metrics
                .entry(event.name)
                .or_default()
                .push(event.duration_us);
        }

        metrics
            .into_iter()
            .map(|(name, mut samples)| {
                samples.sort_unstable();
                let count = samples.len();
                let total: u128 = samples.iter().sum();
                let avg = total / count as u128;
                let p50 = percentile(&samples, 50);
                let p95 = percentile(&samples, 95);
                let max = *samples.last().unwrap_or(&0);
                let slow_count = self
                    .events
                    .iter()
                    .filter(|event| event.name == name && event.slow)
                    .count();
                format!(
                    "{name}: count={count} avg={} p50={} p95={} max={} slow={slow_count}",
                    format_duration_us(avg),
                    format_duration_us(p50),
                    format_duration_us(p95),
                    format_duration_us(max)
                )
            })
            .collect()
    }
}

fn slow_threshold_us(name: &str) -> u128 {
    match name {
        "handle_key" | "handle_mouse" => 1_000,
        "render" | "render_after_lsp" | "syntax_update" => 16_000,
        "finder_preview" | "finder_grep" | "terminal_tick" | "terminal_render" => 5_000,
        "lsp_poll" | "copilot_poll" => 10_000,
        "slow_cycle" => 100_000,
        _ => 10_000,
    }
}

fn format_duration_us(us: u128) -> String {
    if us >= 1_000 {
        let millis = us / 1_000;
        let frac = us % 1_000;
        format!("{millis}.{frac:03}ms")
    } else {
        format!("{us}us")
    }
}

#[cfg(test)]
mod tests {
    use super::{FlightRecorder, PerfStats};
    use std::time::Duration;

    #[test]
    fn profile_summary_reports_sorted_metrics_and_percentiles() {
        let mut stats = PerfStats::default();
        stats.record("render", Duration::from_micros(3000));
        stats.record("render", Duration::from_micros(1000));
        stats.record("render", Duration::from_micros(2000));
        stats.record("handle_key", Duration::from_micros(500));

        assert_eq!(
            stats.summary_lines(),
            vec![
                "handle_key count=1 samples=1 total_us=500 avg_us=500 p50_us=500 p95_us=500 max_us=500",
                "render count=3 samples=3 total_us=6000 avg_us=2000 p50_us=2000 p95_us=3000 max_us=3000",
            ]
        );
    }

    #[test]
    fn profile_summary_writer_includes_header_and_empty_stats_write_nothing() {
        let empty = PerfStats::default();
        let mut output = Vec::new();
        empty.write_summary(&mut output).unwrap();
        assert!(output.is_empty());

        let mut stats = PerfStats::default();
        stats.record("syntax_update", Duration::from_micros(42));

        stats.write_summary(&mut output).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "# profile summary\nsyntax_update count=1 samples=1 total_us=42 avg_us=42 p50_us=42 p95_us=42 max_us=42\n"
        );
    }

    #[test]
    fn profile_summary_caps_retained_samples_without_losing_counts() {
        let mut stats = PerfStats::default();

        for _ in 0..10_005 {
            stats.record("render", Duration::from_micros(1));
        }

        assert_eq!(
            stats.summary_lines(),
            vec![
                "render count=10005 samples=10000 total_us=10005 avg_us=1 p50_us=1 p95_us=1 max_us=1",
            ]
        );
    }

    #[test]
    fn flight_recorder_keeps_recent_events_and_summaries() {
        let mut recorder = FlightRecorder::with_capacity(3);

        recorder.record("render", Duration::from_micros(900));
        recorder.record("handle_key", Duration::from_micros(1_500));
        recorder.record("render", Duration::from_micros(2_100));
        recorder.record("syntax_update", Duration::from_micros(30_000));

        let report = recorder.render_report();

        assert!(report.contains("# Nevi Flight Recorder"));
        assert!(report.contains("Events retained: 3 / 3"));
        assert!(report.contains("handle_key"));
        assert!(report.contains("render"));
        assert!(report.contains("syntax_update"));
        assert!(report.contains("slow"));
        assert!(!report.contains("900us"));
    }

    #[test]
    fn flight_recorder_empty_report_explains_no_events() {
        let recorder = FlightRecorder::with_capacity(8);

        let report = recorder.render_report();

        assert!(report.contains("# Nevi Flight Recorder"));
        assert!(report.contains("No timing events recorded yet."));
    }
}
