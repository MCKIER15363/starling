//! The Stats overlay (Horizon's Overlay setting, `stats`): frames with the 1% low, CPU and GPU
//! time, the process's memory and the GPU's, chunks, meshing and draws, temperatures and
//! throttling, the render distance and the graphics backend, in a panel at the top left. The
//! same numbers are `horizon-ctl perf`'s reply.
use crate::{
    perf_overlay::FrameSummary,
    system_stats::{SystemSample, thermal_name},
    ui_theme::{Ui, color},
};
use serde_json::{Value, json};

/// The game's side of the numbers, gathered each frame (game/perf.rs).
#[derive(Clone, Debug, Default)]
pub struct StatsView {
    pub frames: Option<FrameSummary>,
    pub system: SystemSample,
    /// Device memory the backend's allocator holds (Vulkan): allocated and reserved.
    pub gpu_allocated: Option<u64>,
    pub gpu_reserved: Option<u64>,
    pub chunks: usize,
    pub meshed_sections: u64,
    /// Columns waiting for a worker, and sections being meshed.
    pub streaming_queue: usize,
    pub meshing: usize,
    pub pending_uploads: usize,
    pub draw_calls: usize,
    pub drawn_sections: usize,
    pub render_distance: i32,
    pub backend: String,
    pub gpu: String,
}

const GOOD: u32 = color::hex(0x7ed69a, 255);
const FAIR: u32 = color::hex(0xf0c860, 255);
const BAD: u32 = color::hex(0xf06a5a, 255);

fn mb(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "-".into(), |b| format!("{}", b / (1024 * 1024)))
}

fn celsius(c: Option<f32>) -> String {
    c.map_or_else(|| "-".into(), |c| format!("{c:.0}°C"))
}

impl StatsView {
    /// The panel's lines and their colours.
    pub fn lines(&self) -> Vec<(String, u32)> {
        let s = &self.system;
        let mut lines = Vec::new();
        match self.frames {
            Some(f) => {
                let tint = if f.fps >= 55.0 {
                    GOOD
                } else if f.fps >= 30.0 {
                    FAIR
                } else {
                    BAD
                };
                lines.push((
                    format!(
                        "{:.0} fps   frame {:.1} ms   1% low {:.0} fps",
                        f.fps, f.frame_ms, f.low_1pct_fps
                    ),
                    tint,
                ));
                lines.push((
                    format!(
                        "cpu {:.1} ms   gpu {:.1} ms   worst {:.1} ms",
                        f.cpu_ms, f.gpu_ms, f.worst_ms
                    ),
                    color::TEXT,
                ));
            }
            None => lines.push(("no frames yet".into(), color::MUTED)),
        }
        lines.push((
            format!(
                "RAM rss {} MB   pss {} MB   heap {} MB",
                mb(s.rss_bytes),
                mb(s.pss_bytes),
                mb(s.heap_bytes)
            ),
            color::TEXT,
        ));
        lines.push((
            format!(
                "GPU memory {} MB in use, {} MB reserved",
                mb(self.gpu_allocated),
                mb(self.gpu_reserved)
            ),
            color::TEXT,
        ));
        lines.push((
            format!(
                "chunks {}   meshed {}   queue {}   meshing {}   uploads {}",
                self.chunks,
                self.meshed_sections,
                self.streaming_queue,
                self.meshing,
                self.pending_uploads
            ),
            color::TEXT,
        ));
        lines.push((
            format!(
                "draw calls {} ({} sections)",
                self.draw_calls, self.drawn_sections
            ),
            color::TEXT,
        ));
        let clock = s
            .cpu_mhz
            .map_or_else(String::new, |[cur, max]| format!("   cpu {cur}/{max} MHz"));
        lines.push((
            format!(
                "temp cpu {}  gpu {}  battery {}{clock}",
                celsius(s.cpu_celsius),
                celsius(s.gpu_celsius),
                celsius(s.battery_celsius)
            ),
            color::TEXT,
        ));
        let thermal = match s.thermal_status {
            Some(status) => {
                let headroom = s
                    .thermal_headroom
                    .map_or_else(String::new, |h| format!("   headroom {h:.2}"));
                let tint = match status {
                    0 => GOOD,
                    1 | 2 => FAIR,
                    _ => BAD,
                };
                (format!("thermal {}{headroom}", thermal_name(status)), tint)
            }
            None => ("thermal -".into(), color::MUTED),
        };
        lines.push(thermal);
        lines.push((
            format!(
                "render distance {}   {} · {}",
                self.render_distance, self.backend, self.gpu
            ),
            color::MUTED,
        ));
        lines
    }

    /// The panel is no longer drawn: the stats overlay never appears on screen.
    pub fn draw(&self, _ui: &mut Ui) {}

    /// `horizon-ctl perf`.
    pub fn json(&self) -> Value {
        let s = &self.system;
        json!({
            "fps": self.frames.map(|f| f.fps),
            "frame_ms": self.frames.map(|f| f.frame_ms),
            "worst_ms": self.frames.map(|f| f.worst_ms),
            "low_1pct_fps": self.frames.map(|f| f.low_1pct_fps),
            "cpu_ms": self.frames.map(|f| f.cpu_ms),
            "gpu_ms": self.frames.map(|f| f.gpu_ms),
            "memory": {
                "rss_bytes": s.rss_bytes,
                "pss_bytes": s.pss_bytes,
                "heap_bytes": s.heap_bytes,
                "gpu_allocated_bytes": self.gpu_allocated,
                "gpu_reserved_bytes": self.gpu_reserved,
            },
            "world": {
                "chunks": self.chunks,
                "meshed_sections": self.meshed_sections,
                "streaming_queue": self.streaming_queue,
                "meshing": self.meshing,
                "pending_uploads": self.pending_uploads,
                "draw_calls": self.draw_calls,
                "drawn_sections": self.drawn_sections,
                "render_distance": self.render_distance,
            },
            "thermal": {
                "status": s.thermal_status.map(thermal_name),
                "headroom": s.thermal_headroom,
                "cpu_celsius": s.cpu_celsius,
                "gpu_celsius": s.gpu_celsius,
                "battery_celsius": s.battery_celsius,
                "cpu_mhz": s.cpu_mhz,
            },
            "backend": self.backend,
            "gpu": self.gpu,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_numbers_read_as_dashes_and_known_ones_in_units() {
        let mut view = StatsView {
            render_distance: 8,
            backend: "Vulkan".into(),
            gpu: "Adreno (TM) 740".into(),
            ..Default::default()
        };
        let lines = view.lines();
        assert_eq!(lines[0].0, "no frames yet");
        assert!(
            lines
                .iter()
                .any(|(l, _)| l == "RAM rss - MB   pss - MB   heap - MB")
        );
        assert!(lines.iter().any(|(l, _)| l == "thermal -"));
        view.frames = Some(FrameSummary {
            fps: 59.6,
            frame_ms: 16.8,
            worst_ms: 40.2,
            low_1pct_fps: 31.0,
            cpu_ms: 4.9,
            gpu_ms: 11.7,
        });
        view.system = SystemSample {
            rss_bytes: Some(828 * 1024 * 1024),
            thermal_status: Some(2),
            thermal_headroom: Some(0.81),
            cpu_celsius: Some(54.4),
            cpu_mhz: Some([864, 3187]),
            ..Default::default()
        };
        let lines = view.lines();
        assert_eq!(
            lines[0],
            ("60 fps   frame 16.8 ms   1% low 31 fps".into(), GOOD)
        );
        assert!(lines.iter().any(|(l, _)| l.starts_with("RAM rss 828 MB")));
        assert!(
            lines
                .iter()
                .any(|(l, _)| l == "temp cpu 54°C  gpu -  battery -   cpu 864/3187 MHz")
        );
        assert!(lines.contains(&("thermal moderate   headroom 0.81".into(), FAIR)));
        let json = view.json();
        assert_eq!(json["thermal"]["status"], "moderate");
        assert_eq!(json["world"]["render_distance"], 8);
    }
}
