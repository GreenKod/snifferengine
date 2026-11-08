use sniffer_core::profiler::{FramePhaseMetrics, JankBottleneck, JankWatchdog};
use std::time::{Duration, Instant};

#[test]
fn test_frame_budget_deadlines_at_various_refresh_rates() {
    // 60 Hz -> budget is 16.666 ms
    let w60 = JankWatchdog::new(60.0, 2.0);
    assert!((w60.target_budget_ms() - 16.666_668).abs() < 1e-4);

    // 90 Hz -> budget is 11.111 ms
    let w90 = JankWatchdog::new(90.0, 1.5);
    assert!((w90.target_budget_ms() - 11.111_111).abs() < 1e-4);

    // 120 Hz -> budget is 8.333 ms
    let w120 = JankWatchdog::new(120.0, 1.0);
    assert!((w120.target_budget_ms() - 8.333_333).abs() < 1e-4);

    // 144 Hz -> budget is 6.944 ms
    let w144 = JankWatchdog::new(144.0, 1.0);
    assert!((w144.target_budget_ms() - 6.944_444).abs() < 1e-4);

    // 240 Hz -> budget is 4.166 ms
    let w240 = JankWatchdog::new(240.0, 0.5);
    assert!((w240.target_budget_ms() - 4.166_667).abs() < 1e-4);

    // Clamping invalid / out-of-range refresh rates
    let w_low = JankWatchdog::new(10.0, 2.0);
    assert_eq!(w_low.target_budget_ms(), 1000.0 / 60.0);

    let w_high = JankWatchdog::new(500.0, 2.0);
    assert_eq!(w_high.target_budget_ms(), 1000.0 / 60.0);
}

#[test]
fn test_pipelined_main_and_render_thread_work_latency() {
    // Main thread layout: 5.0ms, Queue: 1.0ms, Render draw: 6.0ms, VSync wait: 4.0ms
    let phases = FramePhaseMetrics {
        layout_and_physics_ms: 5.0,
        queue_latency_ms: 1.0,
        render_draw_ms: 6.0,
        swap_vsync_ms: 4.0,
        total_frame_ms: 16.0,
    };

    // Work latency excluding VSync blocking wait
    assert_eq!(phases.work_latency_ms(), 12.0);

    // In a pipelined architecture, each thread runs in parallel.
    // The busiest thread took 6.0ms, which is well within 60Hz (16.6ms) and 120Hz (8.33ms) budgets!
    assert_eq!(phases.busiest_thread_work_ms(), 6.0);
}

#[test]
fn test_jank_bottleneck_classification() {
    let mut watchdog = JankWatchdog::new(60.0, 2.0);

    // Case 1: Layout & Physics Overrun (Layout takes 25ms, exceeding 16.67ms budget)
    let p_layout = FramePhaseMetrics {
        layout_and_physics_ms: 25.0,
        queue_latency_ms: 0.5,
        render_draw_ms: 4.0,
        swap_vsync_ms: 1.0,
        total_frame_ms: 30.5,
    };
    let jank1 = watchdog.evaluate_frame(1, p_layout).expect("Must jank");
    assert_eq!(jank1.bottleneck, JankBottleneck::LayoutAndPhysics);

    // Case 2: Render & GPU Draw Overrun (Draw takes 25ms)
    let p_draw = FramePhaseMetrics {
        layout_and_physics_ms: 4.0,
        queue_latency_ms: 0.5,
        render_draw_ms: 25.0,
        swap_vsync_ms: 1.0,
        total_frame_ms: 30.5,
    };
    let jank2 = watchdog.evaluate_frame(2, p_draw).expect("Must jank");
    assert_eq!(jank2.bottleneck, JankBottleneck::RenderDraw);

    // Case 3: Queue Latency Overrun (Queue latency causes missed VSync)
    let t0 = Instant::now();
    let p_nominal = FramePhaseMetrics {
        layout_and_physics_ms: 3.0,
        queue_latency_ms: 0.2,
        render_draw_ms: 3.0,
        swap_vsync_ms: 2.0,
        total_frame_ms: 8.2,
    };
    watchdog.evaluate_presented_frame(2, p_nominal, t0, t0 + Duration::from_millis(16));

    let t1 = t0 + Duration::from_millis(16);
    let t1_end = t1 + Duration::from_millis(40);
    let p_queue = FramePhaseMetrics {
        layout_and_physics_ms: 4.0,
        queue_latency_ms: 28.0,
        render_draw_ms: 4.0,
        swap_vsync_ms: 4.0,
        total_frame_ms: 40.0,
    };
    let jank3 = watchdog
        .evaluate_presented_frame(3, p_queue, t1, t1_end)
        .expect("Must jank on queue latency");
    assert_eq!(jank3.bottleneck, JankBottleneck::QueueLatency);

    // Case 4: VSync / Driver Overrun (Swap takes 35ms, exceeding 1.8x budget)
    let p_swap = FramePhaseMetrics {
        layout_and_physics_ms: 4.0,
        queue_latency_ms: 0.5,
        render_draw_ms: 4.0,
        swap_vsync_ms: 35.0,
        total_frame_ms: 43.5,
    };
    let jank4 = watchdog.evaluate_frame(4, p_swap).expect("Must jank");
    assert_eq!(jank4.bottleneck, JankBottleneck::VsyncOrDriver);
}

#[test]
fn test_render_message_queue_coalescing_and_bounded_capacity() {
    #[derive(Debug, PartialEq, Eq)]
    enum TestRenderMsg {
        RequestRedraw,
        LowMemory,
        TerminateWindow,
    }

    let (tx, rx) = crossbeam_channel::bounded::<TestRenderMsg>(16);

    // Simulate 100 rapid RequestRedraw calls (coalesced via try_send)
    let mut dropped_redraws = 0;
    for _ in 0..100 {
        if tx.try_send(TestRenderMsg::RequestRedraw).is_err() {
            dropped_redraws += 1;
        }
    }

    // Capacity is strictly bounded to 16
    assert_eq!(rx.len(), 16);
    assert_eq!(dropped_redraws, 84);

    // Coalesce redraws on the receiver side
    let mut needs_draw = false;
    while let Ok(msg) = rx.try_recv() {
        if msg == TestRenderMsg::RequestRedraw {
            needs_draw = true;
        }
    }

    assert!(needs_draw);
    assert!(rx.is_empty());

    // Lifecycle messages can now be cleanly sent without queue pressure
    assert!(tx.send(TestRenderMsg::LowMemory).is_ok());
    assert!(tx.send(TestRenderMsg::TerminateWindow).is_ok());
    assert_eq!(rx.recv().unwrap(), TestRenderMsg::LowMemory);
    assert_eq!(rx.recv().unwrap(), TestRenderMsg::TerminateWindow);
}

#[test]
fn test_vsync_continuous_simulation_zero_jank_at_120hz() {
    let mut watchdog = JankWatchdog::new(120.0, 1.0);
    let period = Duration::from_nanos(8_333_333); // 120Hz period

    let mut t = Instant::now();
    for frame_idx in 0..120 {
        let frame_start = t;
        let layout_end = frame_start + Duration::from_millis(2);
        let render_start = layout_end + Duration::from_micros(200);
        let _draw_end = render_start + Duration::from_millis(3);
        let swap_end = frame_start + period;

        let phases = FramePhaseMetrics {
            layout_and_physics_ms: 2.0,
            queue_latency_ms: 0.2,
            render_draw_ms: 3.0,
            swap_vsync_ms: 3.133,
            total_frame_ms: 8.333,
        };

        let jank = watchdog.evaluate_presented_frame(frame_idx, phases, frame_start, swap_end);
        assert!(jank.is_none(), "Frame {frame_idx} should not jank");

        t = swap_end;
    }

    assert_eq!(watchdog.jank_count(), 0);
    assert_eq!(watchdog.jank_percentage(), 0.0);
    assert_eq!(watchdog.hitch_time_ratio_ms_per_s(), 0.0);
    assert_eq!(watchdog.total_active_frames(), 120);
}

#[test]
fn test_vsync_missed_deadline_detection_and_attribution() {
    let mut watchdog = JankWatchdog::new(120.0, 1.0);
    let period = Duration::from_nanos(8_333_333);

    // Initial smooth frames
    let t0 = Instant::now();
    let phases_nominal = FramePhaseMetrics {
        layout_and_physics_ms: 2.0,
        queue_latency_ms: 0.2,
        render_draw_ms: 3.0,
        swap_vsync_ms: 3.1,
        total_frame_ms: 8.3,
    };
    watchdog.evaluate_presented_frame(0, phases_nominal, t0, t0 + period);

    // Hitch frame: Render draw took 15ms (exceeding 8.33ms budget)
    let t1 = t0 + period;
    let t1_end = t1 + Duration::from_millis(20);
    let phases_jank = FramePhaseMetrics {
        layout_and_physics_ms: 2.0,
        queue_latency_ms: 0.2,
        render_draw_ms: 15.0,
        swap_vsync_ms: 2.8,
        total_frame_ms: 20.0,
    };

    let jank = watchdog.evaluate_presented_frame(1, phases_jank, t1, t1_end);
    assert!(jank.is_some(), "Hitch frame must be detected as jank");

    let event = jank.unwrap();
    assert_eq!(event.frame_number, 1);
    assert_eq!(event.bottleneck, JankBottleneck::RenderDraw);
    assert!(event.overrun_ms > 0.0);
    assert!(watchdog.has_recent_jank(Duration::from_secs(1)));
    assert_eq!(watchdog.jank_count(), 1);
    assert!(watchdog.hitch_time_ratio_ms_per_s() > 0.0);
}
