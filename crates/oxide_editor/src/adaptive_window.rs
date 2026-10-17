//! Window > Analysis > Adaptive Performance: live thermal state, bottleneck and frame
//! times, the scalers' levels (with manual overrides), and a simulator for thermal
//! warnings and bottlenecks (Unity's Device Simulator Adaptive Performance extension).

use egui::RichText;
use oxide_engine::Engine;
use oxide_engine::adaptive_performance::{
    AdaptivePerformance, AdaptivePerformanceSettings, PerformanceBottleneck, ScalerKind,
    ThermalMetrics, WarningLevel,
};

pub fn window_ui(engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut engine.world;
    let settings = w
        .try_resource::<AdaptivePerformanceSettings>()
        .cloned()
        .unwrap_or_default();
    if w.try_resource::<AdaptivePerformance>().is_none() {
        w.resources.insert(AdaptivePerformance::default());
    }
    let ap = w.resource_mut::<AdaptivePerformance>();
    egui::ScrollArea::vertical().show(ui, |ui| {
        if !settings.enabled {
            ui.label("Adaptive Performance is off (Project Settings > Adaptive Performance).");
        } else if !ap.active() {
            ui.label("Adaptive Performance runs in Play mode. Simulator settings apply then.");
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Status").strong());
        let t = ap.frame_timing();
        let th = ap.thermal_metrics();
        egui::Grid::new("ap-status").num_columns(2).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, k: &str, v: String| {
                ui.label(k);
                ui.label(v);
                ui.end_row();
            };
            row(ui, "Thermal warning", format!("{:?}", th.warning_level));
            row(
                ui,
                "Temperature level",
                format!(
                    "{:.2} (trend {:+.3}/s)",
                    th.temperature_level, th.temperature_trend
                ),
            );
            row(
                ui,
                "Bottleneck",
                format!("{:?}", ap.performance_bottleneck()),
            );
            row(
                ui,
                "Target frame rate",
                format!("{:.0} fps", ap.target_frame_rate()),
            );
            row(
                ui,
                "Frame time",
                format!("{:.2} ms", t.average_frame_time * 1000.0),
            );
            row(
                ui,
                "CPU frame time",
                format!("{:.2} ms", t.average_cpu_frame_time * 1000.0),
            );
            row(
                ui,
                "GPU frame time",
                if t.average_gpu_frame_time > 0.0 {
                    format!("{:.2} ms", t.average_gpu_frame_time * 1000.0)
                } else {
                    "—".into()
                },
            );
            row(
                ui,
                "Indexer",
                format!(
                    "{:?}{}",
                    ap.action(),
                    if settings.indexer.active {
                        ""
                    } else {
                        " (inactive)"
                    }
                ),
            );
        });

        ui.add_space(8.0);
        ui.label(RichText::new("Scalers").strong());
        egui::Grid::new("ap-scalers")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Scaler");
                ui.label("Level");
                ui.label("Scale");
                ui.label("Override");
                ui.end_row();
                for k in ScalerKind::ALL {
                    let cfg = settings.scaler(k);
                    let name = &k.name()["Adaptive".len()..];
                    ui.add_enabled(cfg.enabled, egui::Label::new(name))
                        .on_hover_text(format!(
                            "{} ({:?} visual impact)",
                            k.name(),
                            cfg.visual_impact
                        ));
                    let level = ap.level(k);
                    ui.add(
                        egui::ProgressBar::new(level as f32 / cfg.max_level.max(1) as f32)
                            .desired_width(90.0)
                            .text(format!("{level} / {}", cfg.max_level)),
                    );
                    ui.label(format!("{:.2}", cfg.scale(level)));
                    ui.horizontal(|ui| {
                        let mut on = ap.overridden(k).is_some();
                        if ui.checkbox(&mut on, "").changed() {
                            ap.override_level(k, on.then_some(level));
                        }
                        if let Some(mut v) = ap.overridden(k)
                            && ui
                                .add(egui::Slider::new(&mut v, 0..=cfg.max_level))
                                .changed()
                        {
                            ap.override_level(k, Some(v));
                        }
                    });
                    ui.end_row();
                }
            });

        ui.add_space(8.0);
        ui.label(RichText::new("Simulator").strong());
        let mut sim_thermal = ap.simulation.thermal.is_some();
        if ui
            .checkbox(&mut sim_thermal, "Simulate thermal state")
            .changed()
        {
            ap.simulation.thermal = sim_thermal.then_some(ThermalMetrics::default());
        }
        if let Some(t) = &mut ap.simulation.thermal {
            ui.horizontal(|ui| {
                ui.label("Warning level");
                egui::ComboBox::from_id_salt("ap-sim-warning")
                    .selected_text(format!("{:?}", t.warning_level))
                    .show_ui(ui, |ui| {
                        for l in [
                            WarningLevel::NoWarning,
                            WarningLevel::ThrottlingImminent,
                            WarningLevel::Throttling,
                        ] {
                            ui.selectable_value(&mut t.warning_level, l, format!("{l:?}"));
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Temperature level");
                ui.add(egui::Slider::new(&mut t.temperature_level, 0.0..=1.0));
            });
        }
        let mut sim_b = ap.simulation.bottleneck.is_some();
        if ui.checkbox(&mut sim_b, "Simulate bottleneck").changed() {
            ap.simulation.bottleneck = sim_b.then_some(PerformanceBottleneck::GPU);
        }
        if let Some(b) = &mut ap.simulation.bottleneck {
            egui::ComboBox::from_id_salt("ap-sim-bottleneck")
                .selected_text(format!("{b:?}"))
                .show_ui(ui, |ui| {
                    for v in [
                        PerformanceBottleneck::CPU,
                        PerformanceBottleneck::GPU,
                        PerformanceBottleneck::TargetFrameRate,
                    ] {
                        ui.selectable_value(b, v, format!("{v:?}"));
                    }
                });
        }
        if ui.button("Reset scalers").clicked() {
            ap.reset();
            for k in ScalerKind::ALL {
                ap.override_level(k, None);
            }
        }
    });
    // Live values: keep repainting while it runs.
    if ap.active() {
        ui.ctx().request_repaint();
    }
}
