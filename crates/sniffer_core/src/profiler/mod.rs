//! Zero-cost Frame Time Profiler & Performance Monitor
//!
//! Active under `#[cfg(feature = "devkit")]`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct TouchTelemetry {
    pub active_pointers: usize,
    pub gesture: String,
    pub target_element: String,
    pub touch_x: f32,
    pub touch_y: f32,
    pub touch_to_render_us: f32,
}

/// Detailed breakdown of execution time across frame lifecycle phases.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FramePhaseMetrics {
    /// Time spent on main thread: input dispatch, JS ticks, physics simulation, and layout calculation.
    pub layout_and_physics_ms: f32,
    /// Time spent waiting in channel / cross-thread queue before render thread picked up the state.
    pub queue_latency_ms: f32,
    /// Time spent on render thread drawing UI elements, Kawase blur passes, and batch buffers.
    pub render_draw_ms: f32,
    /// Time spent in `eglSwapBuffers` waiting for hardware display VSync.
    ///
    /// This is *waiting*, not work: a frame that finishes early legitimately blocks
    /// here until the next VSync. It is therefore never used to decide whether a
    /// frame janked, only to attribute a missed VSync that no work phase explains.
    pub swap_vsync_ms: f32,
    /// Total end-to-end frame latency from main-thread frame start to presentation.
    ///
    /// Includes queue and swap waiting, so it measures *latency*, not *pacing*.
    pub total_frame_ms: f32,
}

impl FramePhaseMetrics {
    /// Pure work latency from input to completion of draw (excluding blocking VSync wait in eglSwapBuffers).
    #[must_use]
    pub fn work_latency_ms(&self) -> f32 {
        self.layout_and_physics_ms + self.queue_latency_ms + self.render_draw_ms
    }

    /// Pure work time of the busiest thread (main-thread layout or render-thread draw).
    ///
    /// Main and render threads run in parallel (pipelined), so each one individually
    /// must fit in one VSync period; their sum does not need to.
    #[must_use]
    pub const fn busiest_thread_work_ms(&self) -> f32 {
        self.layout_and_physics_ms.max(self.render_draw_ms)
    }
}

/// Identifies the primary bottleneck phase responsible for a frame deadline overrun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JankBottleneck {
    /// Main thread stalled in input processing, JS runtime execution, or layout recalculation.
    LayoutAndPhysics,
    /// Queue or synchronization bottleneck between main and render threads.
    QueueLatency,
    /// GPU rendering or fragment shader/blur overdraw stall.
    RenderDraw,
    /// Compositor backpressure or VSync missed deadline in `eglSwapBuffers`.
    VsyncOrDriver,
}

impl std::fmt::Display for JankBottleneck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LayoutAndPhysics => write!(f, "Layout & Physics (Main Thread)"),
            Self::QueueLatency => write!(f, "Thread Queue Latency"),
            Self::RenderDraw => write!(f, "GPU Draw & Shaders (Render Thread)"),
            Self::VsyncOrDriver => write!(f, "VSync / Compositor Backpressure"),
        }
    }
}

/// A recorded jank event containing detailed attribution diagnostics.
#[derive(Clone, Debug)]
pub struct JankEvent {
    pub frame_number: u64,
    pub total_ms: f32,
    pub budget_ms: f32,
    pub overrun_ms: f32,
    pub refresh_rate: f32,
    pub bottleneck: JankBottleneck,
    pub bottleneck_ms: f32,
    pub phases: FramePhaseMetrics,
    /// Number of VSync periods skipped between the previous and this presentation
    /// (0 when only a per-thread work overrun was detected).
    pub missed_vsyncs: u32,
    /// Measured present-to-present interval in milliseconds, if known.
    pub present_interval_ms: Option<f32>,
    pub timestamp: Instant,
}

/// Real-time frame pacing watchdog that flags frame hitches.
///
/// A frame is considered janky when either:
/// 1. **Missed VSync** – the present-to-present interval spans more than one
///    VSync period while frames were being produced continuously, or
/// 2. **Thread overrun** – the busiest thread's pure work (layout or draw)
///    exceeds one period plus tolerance.
///
/// Time spent blocked in `eglSwapBuffers` is *not* work and never triggers jank
/// by itself.
#[derive(Clone, Debug)]
pub struct JankWatchdog {
    refresh_rate: f32,
    tolerance_ms: f32,
    total_active_frames: u64,
    jank_frame_count: u64,
    last_jank_event: Option<JankEvent>,
    jank_history: VecDeque<JankEvent>,
    max_history: usize,
    last_present: Option<Instant>,
    missed_vsync_total: u64,
    hitch_time_ms_total: f32,
    paced_time_ms_total: f32,
}

impl Default for JankWatchdog {
    fn default() -> Self {
        Self::new(60.0, 2.0)
    }
}

impl JankWatchdog {
    /// Creates a new `JankWatchdog` with the specified display refresh rate and overrun tolerance.
    #[must_use]
    pub fn new(refresh_rate: f32, tolerance_ms: f32) -> Self {
        Self {
            refresh_rate: if (30.0..=240.0).contains(&refresh_rate) {
                refresh_rate
            } else {
                60.0
            },
            tolerance_ms: tolerance_ms.max(0.5),
            total_active_frames: 0,
            jank_frame_count: 0,
            last_jank_event: None,
            jank_history: VecDeque::with_capacity(32),
            max_history: 32,
            last_present: None,
            missed_vsync_total: 0,
            hitch_time_ms_total: 0.0,
            paced_time_ms_total: 0.0,
        }
    }

    /// Updates the target hardware refresh rate in Hz (e.g. 60.0, 90.0, 120.0).
    pub fn set_refresh_rate(&mut self, refresh_rate: f32) {
        if (30.0..=240.0).contains(&refresh_rate) {
            self.refresh_rate = refresh_rate;
        }
    }

    /// Returns the target hardware refresh rate in Hz.
    #[must_use]
    pub fn refresh_rate(&self) -> f32 {
        self.refresh_rate
    }

    /// Returns the single-frame time budget in milliseconds (e.g. 11.11ms for 90Hz).
    #[must_use]
    pub fn target_budget_ms(&self) -> f32 {
        1000.0 / self.refresh_rate
    }

    /// Returns the configured tolerance threshold in milliseconds.
    #[must_use]
    pub fn tolerance_ms(&self) -> f32 {
        self.tolerance_ms
    }

    /// Returns the absolute jank deadline in milliseconds (budget + tolerance).
    #[must_use]
    pub fn deadline_ms(&self) -> f32 {
        self.target_budget_ms() + self.tolerance_ms
    }

    /// Evaluates a frame using only its phase timings (no presentation timestamp).
    ///
    /// Only per-thread work overruns can be detected here; use
    /// [`Self::evaluate_presented_frame`] to also detect missed VSyncs.
    pub fn evaluate_frame(
        &mut self,
        frame_number: u64,
        phases: FramePhaseMetrics,
    ) -> Option<JankEvent> {
        self.classify(frame_number, phases, 0, None)
    }

    /// Evaluates a frame using its phase timings **and** its presentation timestamp.
    ///
    /// * `frame_start` – when the main thread started producing this frame.
    /// * `present` – when the frame was handed to the display (`eglSwapBuffers` return).
    ///
    /// The present-to-present interval is only judged when frames are produced
    /// continuously (this frame's work started no later than one period after the
    /// previous presentation). Gaps caused by intentional idling are ignored.
    pub fn evaluate_presented_frame(
        &mut self,
        frame_number: u64,
        phases: FramePhaseMetrics,
        frame_start: Instant,
        present: Instant,
    ) -> Option<JankEvent> {
        let period_ms = self.target_budget_ms();
        let mut missed_vsyncs = 0;
        let mut present_interval_ms = None;

        if let Some(prev_present) = self.last_present {
            let idle_gap_ms = frame_start
                .saturating_duration_since(prev_present)
                .as_secs_f32()
                * 1000.0;
            let is_continuous = idle_gap_ms <= period_ms;

            if is_continuous {
                let interval_ms = present
                    .saturating_duration_since(prev_present)
                    .as_secs_f32()
                    * 1000.0;
                missed_vsyncs = Self::missed_vsyncs_for_interval(interval_ms, period_ms);

                self.paced_time_ms_total += interval_ms;
                #[allow(clippy::cast_precision_loss)]
                {
                    self.hitch_time_ms_total += missed_vsyncs as f32 * period_ms;
                }
                self.missed_vsync_total = self
                    .missed_vsync_total
                    .wrapping_add(u64::from(missed_vsyncs));
                present_interval_ms = Some(interval_ms);
            }
        }
        self.last_present = Some(present);

        self.classify(frame_number, phases, missed_vsyncs, present_interval_ms)
    }

    /// Records a presentation that is not evaluated (e.g. a redraw caused only by a
    /// texture upload) so the next evaluated frame measures its interval from the
    /// correct previous VSync instead of reporting a phantom miss.
    pub const fn note_present(&mut self, present: Instant) {
        self.last_present = Some(present);
    }

    /// Converts a present-to-present interval into the number of skipped VSyncs.
    ///
    /// The interval is rounded to the nearest whole number of periods so that
    /// normal timestamp jitter (e.g. 18ms at 60Hz) is not reported as a miss.
    #[must_use]
    pub fn missed_vsyncs_for_interval(interval_ms: f32, period_ms: f32) -> u32 {
        if period_ms <= 0.0 || !interval_ms.is_finite() || interval_ms <= 0.0 {
            return 0;
        }
        let periods = (interval_ms / period_ms).round().clamp(0.0, 1000.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let periods = periods as u32;
        periods.saturating_sub(1)
    }

    fn classify(
        &mut self,
        frame_number: u64,
        phases: FramePhaseMetrics,
        missed_vsyncs: u32,
        present_interval_ms: Option<f32>,
    ) -> Option<JankEvent> {
        self.total_active_frames = self.total_active_frames.wrapping_add(1);
        let budget_ms = self.target_budget_ms();
        let deadline = budget_ms + self.tolerance_ms;
        let thread_work_ms = phases.busiest_thread_work_ms();
        let thread_overrun =
            phases.layout_and_physics_ms > deadline || phases.render_draw_ms > deadline;
        let swap_stall = phases.swap_vsync_ms > budget_ms * 1.8;

        if missed_vsyncs == 0 && !thread_overrun && !swap_stall {
            return None;
        }

        self.jank_frame_count = self.jank_frame_count.wrapping_add(1);
        let (bottleneck, bottleneck_ms) = Self::attribute(&phases, budget_ms);
        #[allow(clippy::cast_precision_loss)]
        let overrun_ms = if missed_vsyncs > 0 {
            missed_vsyncs as f32 * budget_ms
        } else if swap_stall {
            phases.swap_vsync_ms - budget_ms
        } else {
            thread_work_ms - budget_ms
        };

        let event = JankEvent {
            frame_number,
            total_ms: phases.total_frame_ms,
            budget_ms,
            overrun_ms,
            refresh_rate: self.refresh_rate,
            bottleneck,
            bottleneck_ms,
            phases,
            missed_vsyncs,
            present_interval_ms,
            timestamp: Instant::now(),
        };

        crate::dev_warn!(
            "[JANK WATCHDOG] ⚠️ Frame #{} missed {} vsync(s), +{:.1}ms (interval: {}, budget: {:.2}ms @ {:.0}Hz) | Bottleneck: {} ({:.2}ms) | Work [Layout: {:.1}ms, Draw: {:.1}ms] Wait [Queue: {:.1}ms, Swap: {:.1}ms] Latency: {:.1}ms",
            event.frame_number,
            event.missed_vsyncs,
            event.overrun_ms,
            event
                .present_interval_ms
                .map_or_else(|| "n/a".to_string(), |v| format!("{v:.1}ms")),
            event.budget_ms,
            event.refresh_rate,
            event.bottleneck,
            event.bottleneck_ms,
            event.phases.layout_and_physics_ms,
            event.phases.render_draw_ms,
            event.phases.queue_latency_ms,
            event.phases.swap_vsync_ms,
            event.total_ms
        );

        if self.jank_history.len() >= self.max_history {
            self.jank_history.pop_front();
        }
        self.jank_history.push_back(event.clone());
        self.last_jank_event = Some(event.clone());
        Some(event)
    }

    /// Picks the phase responsible for a janky frame.
    ///
    /// Work phases are blamed first. Queue latency is blamed when the render thread
    /// was still busy with the previous frame (pipeline backpressure). Only when
    /// neither explains the miss is it attributed to VSync / compositor / driver.
    fn attribute(phases: &FramePhaseMetrics, budget_ms: f32) -> (JankBottleneck, f32) {
        let layout = phases.layout_and_physics_ms;
        let draw = phases.render_draw_ms;
        let queue = phases.queue_latency_ms;
        let swap = phases.swap_vsync_ms;

        if draw > budget_ms && draw >= layout {
            (JankBottleneck::RenderDraw, draw)
        } else if layout > budget_ms {
            (JankBottleneck::LayoutAndPhysics, layout)
        } else if queue > budget_ms * 1.2 && queue >= draw && queue >= layout {
            (JankBottleneck::QueueLatency, queue)
        } else if swap > budget_ms * 1.8 {
            (JankBottleneck::VsyncOrDriver, swap)
        } else if draw >= layout {
            (JankBottleneck::RenderDraw, draw)
        } else {
            (JankBottleneck::LayoutAndPhysics, layout)
        }
    }

    /// Returns the percentage of active frames that missed their presentation deadline.
    #[must_use]
    pub fn jank_percentage(&self) -> f32 {
        if self.total_active_frames == 0 {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            let ratio = (self.jank_frame_count as f32 / self.total_active_frames as f32) * 100.0;
            ratio.clamp(0.0, 100.0)
        }
    }

    /// Returns `true` if a jank event occurred within the specified recent duration.
    #[must_use]
    pub fn has_recent_jank(&self, duration: Duration) -> bool {
        self.last_jank_event
            .as_ref()
            .is_some_and(|ev| ev.timestamp.elapsed() < duration)
    }

    /// Returns a reference to the most recent jank event, if one occurred.
    #[must_use]
    pub fn last_jank(&self) -> Option<&JankEvent> {
        self.last_jank_event.as_ref()
    }

    /// Returns the total count of active frames evaluated.
    #[must_use]
    pub fn total_active_frames(&self) -> u64 {
        self.total_active_frames
    }

    /// Returns the total count of jank frames detected.
    #[must_use]
    pub fn jank_count(&self) -> u64 {
        self.jank_frame_count
    }

    /// Returns the total number of VSync periods skipped during continuous rendering.
    #[must_use]
    pub const fn missed_vsync_count(&self) -> u64 {
        self.missed_vsync_total
    }

    /// Returns the hitch time ratio in milliseconds of delay per second of
    /// continuous rendering (Apple's metric: < 5 good, 5–10 warning, > 10 critical).
    #[must_use]
    pub fn hitch_time_ratio_ms_per_s(&self) -> f32 {
        if self.paced_time_ms_total <= 0.0 {
            0.0
        } else {
            self.hitch_time_ms_total / (self.paced_time_ms_total / 1000.0)
        }
    }
}

#[cfg(feature = "devkit")]
pub struct FrameProfiler {
    frame_times: VecDeque<f32>,
    cpu_times: VecDeque<f32>,
    gpu_times: VecDeque<f32>,
    swap_times: VecDeque<f32>,
    max_history: usize,
    pub watchdog: JankWatchdog,
    pub touch_telemetry: TouchTelemetry,
    pub rendered_entities: usize,
    pub total_entities: usize,
}

#[cfg(feature = "devkit")]
impl FrameProfiler {
    #[must_use]
    pub fn new(max_history: usize) -> Self {
        Self {
            frame_times: VecDeque::with_capacity(max_history),
            cpu_times: VecDeque::with_capacity(max_history),
            gpu_times: VecDeque::with_capacity(max_history),
            swap_times: VecDeque::with_capacity(max_history),
            max_history,
            watchdog: JankWatchdog::default(),
            touch_telemetry: TouchTelemetry::default(),
            rendered_entities: 0,
            total_entities: 0,
        }
    }

    pub fn record_entities(&mut self, rendered: usize, total: usize) {
        self.rendered_entities = rendered;
        self.total_entities = total;
    }

    /// Records comprehensive 5-phase frame metrics and evaluates the jank watchdog.
    pub fn record_frame_detailed(
        &mut self,
        frame_start: Instant,
        layout_end: Instant,
        render_start: Instant,
        draw_end: Instant,
        swap_end: Instant,
        frame_number: u64,
    ) {
        let layout_and_physics_ms = layout_end.duration_since(frame_start).as_secs_f32() * 1000.0;
        let queue_latency_ms = render_start.duration_since(layout_end).as_secs_f32() * 1000.0;
        let render_draw_ms = draw_end.duration_since(render_start).as_secs_f32() * 1000.0;
        let swap_vsync_ms = swap_end.duration_since(draw_end).as_secs_f32() * 1000.0;
        let total_frame_ms = swap_end.duration_since(frame_start).as_secs_f32() * 1000.0;

        let phases = FramePhaseMetrics {
            layout_and_physics_ms,
            queue_latency_ms,
            render_draw_ms,
            swap_vsync_ms,
            total_frame_ms,
        };

        self.watchdog
            .evaluate_presented_frame(frame_number, phases, frame_start, swap_end);

        let push_limit = |vec: &mut VecDeque<f32>, val: f32, limit: usize| {
            if vec.len() >= limit {
                vec.pop_front();
            }
            vec.push_back(val);
        };

        push_limit(&mut self.frame_times, total_frame_ms, self.max_history);
        push_limit(&mut self.cpu_times, layout_and_physics_ms, self.max_history);
        push_limit(&mut self.gpu_times, render_draw_ms, self.max_history);
        push_limit(&mut self.swap_times, swap_vsync_ms, self.max_history);
    }

    /// Backward-compatible convenience wrapper for 4-timestamp frame recording.
    pub fn record_frame(
        &mut self,
        frame_start: Instant,
        render_start: Instant,
        draw_end: Instant,
        swap_end: Instant,
    ) {
        self.record_frame_detailed(
            frame_start,
            render_start,
            render_start,
            draw_end,
            swap_end,
            0,
        );
    }

    #[allow(clippy::cast_precision_loss)]
    fn avg(vec: &VecDeque<f32>) -> f32 {
        if vec.is_empty() {
            return 0.0;
        }
        let sum: f32 = vec.iter().sum();
        sum / (vec.len() as f32)
    }

    #[must_use]
    pub fn average_frame_time_ms(&self) -> f32 {
        Self::avg(&self.frame_times)
    }

    #[must_use]
    pub fn avg_cpu_ms(&self) -> f32 {
        Self::avg(&self.cpu_times)
    }

    #[must_use]
    pub fn avg_gpu_ms(&self) -> f32 {
        Self::avg(&self.gpu_times)
    }

    #[must_use]
    pub fn avg_swap_ms(&self) -> f32 {
        Self::avg(&self.swap_times)
    }

    #[must_use]
    pub fn current_fps(&self) -> f32 {
        let avg = self.average_frame_time_ms();
        if avg > 0.0 {
            (1000.0 / avg).min(120.0)
        } else {
            0.0
        }
    }

    #[must_use]
    pub fn jank_count_120hz(&self) -> usize {
        self.frame_times.iter().filter(|&&t| t > 8.333).count()
    }
}

#[cfg(feature = "devkit")]
impl Default for FrameProfiler {
    fn default() -> Self {
        Self::new(30)
    }
}

#[cfg(not(feature = "devkit"))]
#[derive(Clone, Debug, Default)]
pub struct FrameProfiler {
    pub watchdog: JankWatchdog,
    pub touch_telemetry: TouchTelemetry,
    pub rendered_entities: usize,
    pub total_entities: usize,
}

#[cfg(not(feature = "devkit"))]
impl FrameProfiler {
    #[inline]
    #[must_use]
    pub fn new(_max_history: usize) -> Self {
        Self {
            watchdog: JankWatchdog::default(),
            touch_telemetry: TouchTelemetry::default(),
            rendered_entities: 0,
            total_entities: 0,
        }
    }

    #[inline]
    pub fn record_entities(&mut self, _rendered: usize, _total: usize) {}

    /// Evaluates jank watchdog in release mode without allocating frame history buffers.
    pub fn record_frame_detailed(
        &mut self,
        frame_start: Instant,
        layout_end: Instant,
        render_start: Instant,
        draw_end: Instant,
        swap_end: Instant,
        frame_number: u64,
    ) {
        let layout_and_physics_ms = layout_end.duration_since(frame_start).as_secs_f32() * 1000.0;
        let queue_latency_ms = render_start.duration_since(layout_end).as_secs_f32() * 1000.0;
        let render_draw_ms = draw_end.duration_since(render_start).as_secs_f32() * 1000.0;
        let swap_vsync_ms = swap_end.duration_since(draw_end).as_secs_f32() * 1000.0;
        let total_frame_ms = swap_end.duration_since(frame_start).as_secs_f32() * 1000.0;

        let phases = FramePhaseMetrics {
            layout_and_physics_ms,
            queue_latency_ms,
            render_draw_ms,
            swap_vsync_ms,
            total_frame_ms,
        };

        self.watchdog
            .evaluate_presented_frame(frame_number, phases, frame_start, swap_end);
    }

    #[inline]
    pub fn record_frame(
        &mut self,
        _frame_start: Instant,
        _render_start: Instant,
        _draw_end: Instant,
        _swap_end: Instant,
    ) {
    }

    #[inline]
    #[must_use]
    pub fn average_frame_time_ms(&self) -> f32 {
        0.0
    }

    #[inline]
    #[must_use]
    pub fn avg_cpu_ms(&self) -> f32 {
        0.0
    }

    #[inline]
    #[must_use]
    pub fn avg_gpu_ms(&self) -> f32 {
        0.0
    }

    #[inline]
    #[must_use]
    pub fn avg_swap_ms(&self) -> f32 {
        0.0
    }

    #[inline]
    #[must_use]
    pub fn current_fps(&self) -> f32 {
        0.0
    }

    #[inline]
    #[must_use]
    pub fn jank_count_120hz(&self) -> usize {
        0
    }
}

#[cfg(feature = "devkit")]
pub fn count_elements(element: &crate::types::Element) -> usize {
    match element {
        crate::types::Element::Container { children, .. }
        | crate::types::Element::ScrollView { children, .. }
        | crate::types::Element::SharedView { children, .. } => {
            let mut count = 1;
            for child in children {
                count += count_elements(child);
            }
            count
        }
        _ => 1,
    }
}

#[cfg(not(feature = "devkit"))]
#[inline]
#[must_use]
pub fn count_elements(_element: &crate::types::Element) -> usize {
    0
}

#[cfg(feature = "devkit")]
fn draw_touch_hitboxes(
    renderer: &mut dyn crate::render::Renderer,
    element: &crate::types::Element,
    layout: &crate::layout::LayoutNode,
    parent_opacity: f32,
) {
    let effective_opacity = parent_opacity * element.style().opacity;
    if effective_opacity < 0.05 {
        return;
    }

    let elem_id = element.id();
    let is_button = match elem_id {
        Some(k) => {
            !crate::ui::widget::is_structural_layout_id(k)
                && (k.starts_with("app_item_")
                    || k.starts_with("dock_app_btn_")
                    || k.starts_with("dock_app_circle_")
                    || k.starts_with("fab_")
                    || k.starts_with("app_card_")
                    || k.starts_with("btn_")
                    || k.starts_with("action_")
                    || k.starts_with("clock_")
                    || k.contains("btn")
                    || k.contains("card")
                    || k.contains("item"))
        }
        None => matches!(
            element,
            crate::types::Element::TextInput { .. } | crate::types::Element::Checkbox { .. }
        ),
    };

    let tx = element.style().transform.translate_x;
    let ty = element.style().transform.translate_y;
    let safe_tx = if tx.is_nan() || tx.is_infinite() {
        0.0
    } else {
        tx
    };
    let safe_ty = if ty.is_nan() || ty.is_infinite() {
        0.0
    } else {
        ty
    };
    let has_tx = safe_tx.abs() > 0.001 || safe_ty.abs() > 0.001;
    let is_scroll_view = matches!(element, crate::types::Element::ScrollView { .. });

    if has_tx && !is_scroll_view {
        renderer.push_transform(0.0, 0.0, 1.0, 0.0, safe_tx, safe_ty);
    }

    if is_button && layout.rect.width > 2.0 && layout.rect.height > 2.0 {
        let is_dock = elem_id.as_ref().is_some_and(|k| k.contains("dock"));
        let (fill_color, border_color) = if is_dock {
            (0x284A_DE80_u32, Some(0xEE4A_DE80_u32))
        } else {
            (0x2038_BDF8_u32, Some(0xDD38_BDF8_u32))
        };

        renderer.draw_rect(layout.rect, fill_color, 4.0, 1.5, border_color);
    }

    match element {
        crate::types::Element::ScrollView {
            children,
            scroll_x,
            scroll_y,
            ..
        } => {
            let safe_scroll_y = if scroll_y.is_nan() || scroll_y.is_infinite() {
                0.0
            } else {
                *scroll_y
            };
            let safe_scroll_x = if scroll_x.is_nan() || scroll_x.is_infinite() {
                0.0
            } else {
                *scroll_x
            };
            renderer.push_clip_rect(layout.rect, 8.0);
            renderer.push_transform(
                0.0,
                0.0,
                1.0,
                0.0,
                safe_tx - safe_scroll_x,
                safe_ty - safe_scroll_y,
            );
            for (child_el, child_lay) in children.iter().zip(layout.children.iter()) {
                draw_touch_hitboxes(renderer, child_el, child_lay, effective_opacity);
            }
            renderer.pop_transform();
            renderer.pop_clip_rect();
        }
        crate::types::Element::Container { children, .. }
        | crate::types::Element::SharedView { children, .. } => {
            for (child_el, child_lay) in children.iter().zip(layout.children.iter()) {
                draw_touch_hitboxes(renderer, child_el, child_lay, effective_opacity);
            }
        }
        _ => {}
    }

    if has_tx && !is_scroll_view {
        renderer.pop_transform();
    }
}

/// Renders DevKit visualizer debug overlays: touch hitboxes and active pointer circles.
#[cfg(feature = "devkit")]
pub fn render_devkit_debug_overlays(
    renderer: &mut dyn crate::render::Renderer,
    profiler: &FrameProfiler,
    root_element: &crate::types::Element,
    layout_tree: &crate::layout::LayoutNode,
) {
    static SHOW_DEBUG_OVERLAYS: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static CHECK_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    // Periodically re-check (every 60 frames) so adb setprop dynamically toggles overlays
    let count = CHECK_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if count % 60 == 0 {
        #[allow(unused_mut)]
        let mut enabled = std::env::var("SNIFFER_DEBUG_HITBOXES")
            .or_else(|_| std::env::var("SNIFFER_DEBUG_OVERLAYS"))
            .is_ok_and(|val| val == "1" || val.eq_ignore_ascii_case("true"));

        #[cfg(target_os = "android")]
        {
            unsafe extern "C" {
                fn __system_property_get(
                    name: *const std::ffi::c_char,
                    value: *mut std::ffi::c_char,
                ) -> std::ffi::c_int;
            }
            let check_prop = |prop_name: &[u8]| -> Option<bool> {
                let mut buf = [0u8; 92];
                let len = unsafe {
                    __system_property_get(prop_name.as_ptr().cast(), buf.as_mut_ptr().cast())
                };
                if len > 0 {
                    let s = std::str::from_utf8(&buf[..len as usize]).ok()?;
                    Some(s == "1" || s.eq_ignore_ascii_case("true"))
                } else {
                    None
                }
            };
            if let Some(prop_val) = check_prop(b"debug.sniffer.overlays\0")
                .or_else(|| check_prop(b"debug.sniffer.hitboxes\0"))
            {
                enabled = prop_val;
            }
        }

        SHOW_DEBUG_OVERLAYS.store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    if !SHOW_DEBUG_OVERLAYS.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    // 1. Draw touch & hitbox bounding areas
    draw_touch_hitboxes(renderer, root_element, layout_tree, 1.0);

    // 2. Active Pointer / Touch Telemetry Visualizer
    if profiler.touch_telemetry.touch_x > 0.0 || profiler.touch_telemetry.touch_y > 0.0 {
        let tx = profiler.touch_telemetry.touch_x;
        let ty = profiler.touch_telemetry.touch_y;
        renderer.draw_circle(tx, ty, 20.0, 0x44FB_BF24);
        renderer.draw_circle(tx, ty, 10.0, 0x88FB_BF24);
        renderer.draw_circle(tx, ty, 4.0, 0xFFF5_9E0B);
    }

    // 3. Performance & Jank Watchdog HUD Overlay Badge
    let current_fps = profiler.current_fps();
    let jank_pct = profiler.watchdog.jank_percentage();
    let has_recent_jank = profiler
        .watchdog
        .has_recent_jank(std::time::Duration::from_millis(800));

    let badge_x = 24.0;
    let badge_y = 48.0;
    let badge_w = 210.0;
    let badge_h = 32.0;

    let (bg_color, border_color) = if has_recent_jank {
        (0xDD7F_1D1D, 0xFFEF_4444)
    } else {
        (0xCC0F_172A, 0xFF10_B981)
    };

    renderer.draw_rect(
        crate::Rect::new(badge_x, badge_y, badge_w, badge_h),
        bg_color,
        8.0,
        1.5,
        Some(border_color),
    );

    let dot_color = if has_recent_jank {
        0xFFEF_4444
    } else {
        0xFF10_B981
    };
    renderer.draw_circle(badge_x + 14.0, badge_y + 16.0, 4.5, dot_color);

    let status_text = if has_recent_jank {
        if let Some(jank) = profiler.watchdog.last_jank() {
            let label = match jank.bottleneck {
                JankBottleneck::LayoutAndPhysics => "Layout",
                JankBottleneck::QueueLatency => "Queue",
                JankBottleneck::RenderDraw => "GPU",
                JankBottleneck::VsyncOrDriver => "VSync",
            };
            format!("JANK +{:.1}ms ({label})", jank.overrun_ms)
        } else {
            "JANK DETECTED".to_string()
        }
    } else {
        format!("{current_fps:.0} FPS ({jank_pct:.1}% jank)")
    };

    renderer.draw_text(
        &status_text,
        badge_x + 26.0,
        badge_y + 20.0,
        12.0,
        0xFFFF_FFFF,
    );
}

/// Renders DevKit visualizer debug overlays (no-op when devkit feature is disabled).
#[cfg(not(feature = "devkit"))]
#[inline]
pub fn render_devkit_debug_overlays(
    _renderer: &mut dyn crate::render::Renderer,
    _profiler: &FrameProfiler,
    _root_element: &crate::types::Element,
    _layout_tree: &crate::layout::LayoutNode,
) {
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jank_watchdog_nominal_frame_no_jank() {
        let mut watchdog = JankWatchdog::new(90.0, 2.0);
        assert_eq!(watchdog.target_budget_ms(), 1000.0 / 90.0);
        assert_eq!(watchdog.deadline_ms(), (1000.0 / 90.0) + 2.0);

        let phases = FramePhaseMetrics {
            layout_and_physics_ms: 2.0,
            queue_latency_ms: 0.5,
            render_draw_ms: 2.0,
            swap_vsync_ms: 4.0,
            total_frame_ms: 8.5,
        };

        let jank = watchdog.evaluate_frame(1, phases);
        assert!(jank.is_none());
        assert_eq!(watchdog.jank_count(), 0);
        assert_eq!(watchdog.total_active_frames(), 1);
        assert_eq!(watchdog.jank_percentage(), 0.0);
    }

    #[test]
    fn test_jank_watchdog_detects_layout_bottleneck() {
        let mut watchdog = JankWatchdog::new(90.0, 2.0);
        let phases = FramePhaseMetrics {
            layout_and_physics_ms: 14.0,
            queue_latency_ms: 0.5,
            render_draw_ms: 2.0,
            swap_vsync_ms: 3.5,
            total_frame_ms: 20.0,
        };

        let jank = watchdog.evaluate_frame(2, phases);
        assert!(jank.is_some());
        let event = jank.unwrap();
        assert_eq!(event.frame_number, 2);
        assert_eq!(event.bottleneck, JankBottleneck::LayoutAndPhysics);
        assert_eq!(event.bottleneck_ms, 14.0);
        assert!(event.overrun_ms > 0.0);
        assert_eq!(watchdog.jank_count(), 1);
        assert_eq!(watchdog.jank_percentage(), 100.0);
        assert!(watchdog.has_recent_jank(Duration::from_secs(1)));
    }

    #[test]
    fn test_jank_watchdog_detects_gpu_bottleneck() {
        let mut watchdog = JankWatchdog::new(60.0, 2.0);
        let phases = FramePhaseMetrics {
            layout_and_physics_ms: 1.0,
            queue_latency_ms: 0.5,
            render_draw_ms: 19.5,
            swap_vsync_ms: 4.0,
            total_frame_ms: 25.0,
        };

        let jank = watchdog.evaluate_frame(3, phases);
        assert!(jank.is_some());
        let event = jank.unwrap();
        assert_eq!(event.bottleneck, JankBottleneck::RenderDraw);
        assert_eq!(event.bottleneck_ms, 19.5);
    }

    #[test]
    fn test_jank_watchdog_pipelined_queue_wait_not_jank() {
        let mut watchdog = JankWatchdog::new(60.0, 2.0);
        let phases = FramePhaseMetrics {
            layout_and_physics_ms: 0.5,
            queue_latency_ms: 16.0,
            render_draw_ms: 6.0,
            swap_vsync_ms: 9.0,
            total_frame_ms: 31.5,
        };

        let jank = watchdog.classify(10, phases, 0, Some(16.4));
        assert!(
            jank.is_none(),
            "Normal pipelined queue wait with 0 missed VSyncs must not be flagged as jank"
        );
    }
}
