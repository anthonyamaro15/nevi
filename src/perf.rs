use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::Duration;

const MAX_SAMPLES_PER_METRIC: usize = 10_000;

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

#[cfg(test)]
mod tests {
    use super::PerfStats;
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
}
