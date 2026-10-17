//! Window → General → Test Runner (Unity's Test Runner): the project's play-mode tests
//! (`scripts! { ...; tests: ... }`, run one by one in Play mode, each on a fresh scene) and
//! edit-mode tests (`#[test]` functions in the Scripts crate, run with `cargo test`), with
//! pass / fail, durations and failure messages.
//! <https://docs.unity3d.com/Packages/com.unity.test-framework@1.4/manual/workflow-run-test.html>

use egui::{Color32, RichText};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

/// Play-mode tests fail after this long.
const PLAY_TIMEOUT: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum TestStatus {
    NotRun,
    Running,
    Passed,
    Failed,
    Ignored,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TestRow {
    pub name: String,
    pub status: TestStatus,
    /// Seconds (play-mode tests).
    pub seconds: Option<f32>,
    pub message: String,
}

impl TestRow {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            status: TestStatus::NotRun,
            seconds: None,
            message: String::new(),
        }
    }
}

/// A play-mode run in progress.
struct PlayRun {
    queue: VecDeque<String>,
    current: Option<(String, Instant, oxide_scene::testing::TestOutcome)>,
    /// Whether the runner entered Play mode (and so leaves it at the end).
    entered_play: bool,
}

/// What `cargo test` reported.
struct EditOutcome {
    rows: Vec<TestRow>,
    seconds: f32,
    error: Option<String>,
}

#[derive(Default)]
pub struct TestRunner {
    /// 0 PlayMode, 1 EditMode.
    pub tab: usize,
    pub play: Vec<TestRow>,
    pub edit: Vec<TestRow>,
    selected: Option<String>,
    play_run: Option<PlayRun>,
    edit_job: Option<(Receiver<EditOutcome>, Instant)>,
    pub status: String,
}

impl TestRunner {
    pub fn running(&self) -> bool {
        self.play_run.is_some() || self.edit_job.is_some()
    }

    /// Keeps the play-mode list in step with the loaded Scripts library.
    pub fn sync_play_tests(&mut self, names: &[&'static str]) {
        if self.play_run.is_some() {
            return;
        }
        let same = self.play.len() == names.len()
            && self.play.iter().zip(names).all(|(r, n)| r.name == *n);
        if !same {
            self.play = names
                .iter()
                .map(|n| {
                    self.play
                        .iter()
                        .find(|r| r.name == *n)
                        .cloned()
                        .unwrap_or_else(|| TestRow::new(n))
                })
                .collect();
        }
    }

    /// Starts running play-mode tests (`None`: all).
    pub fn run_play(&mut self, only: Option<Vec<String>>) {
        if self.running() {
            return;
        }
        let queue: VecDeque<String> = self
            .play
            .iter()
            .filter(|r| only.as_ref().is_none_or(|o| o.contains(&r.name)))
            .map(|r| r.name.clone())
            .collect();
        for r in &mut self.play {
            if queue.contains(&r.name) {
                r.status = TestStatus::Running;
                r.message.clear();
                r.seconds = None;
            }
        }
        if queue.is_empty() {
            self.status = "No play-mode tests to run".into();
            return;
        }
        self.status = format!("Running {} play-mode test(s)…", queue.len());
        self.play_run = Some(PlayRun {
            queue,
            current: None,
            entered_play: false,
        });
    }

    /// Starts `cargo test` on the Scripts crate in the background (`filter`: a name filter).
    pub fn run_edit(&mut self, project: PathBuf, filter: Option<String>) {
        if self.running() {
            return;
        }
        for r in &mut self.edit {
            if filter.as_ref().is_none_or(|f| r.name.contains(f.as_str())) {
                r.status = TestStatus::Running;
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("edit-mode tests".into())
            .spawn(move || {
                let _ = tx.send(cargo_test(&project, filter.as_deref()));
            });
        match spawned {
            Ok(_) => {
                self.edit_job = Some((rx, Instant::now()));
                self.status = "Compiling and running edit-mode tests…".into();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// Picks up finished edit-mode runs.
    pub fn poll_edit(&mut self) {
        let Some((rx, _)) = &self.edit_job else {
            return;
        };
        let Ok(out) = rx.try_recv() else {
            return;
        };
        self.edit_job = None;
        match out.error {
            Some(e) => {
                self.status = format!("Edit-mode tests didn't run: {e}");
                for r in &mut self.edit {
                    if r.status == TestStatus::Running {
                        r.status = TestStatus::NotRun;
                    }
                }
            }
            None => {
                let failed = out
                    .rows
                    .iter()
                    .filter(|r| r.status == TestStatus::Failed)
                    .count();
                self.status = format!(
                    "{} edit-mode test(s), {failed} failed, in {:.1} s",
                    out.rows.len(),
                    out.seconds
                );
                // Results replace rows of the same name; tests not run keep theirs.
                for row in out.rows {
                    match self.edit.iter_mut().find(|r| r.name == row.name) {
                        Some(r) => *r = row,
                        None => self.edit.push(row),
                    }
                }
                for r in &mut self.edit {
                    if r.status == TestStatus::Running {
                        r.status = TestStatus::NotRun;
                    }
                }
                self.edit.sort_by(|a, b| a.name.cmp(&b.name));
            }
        }
    }
}

/// Runs `cargo test` on the project's Scripts crate and parses libtest's output.
fn cargo_test(project: &std::path::Path, filter: Option<&str>) -> EditOutcome {
    let started = Instant::now();
    let mut cmd = std::process::Command::new(crate::project_files::cargo_command());
    cmd.args(["test", "-p", crate::scripting::SCRIPTS_CRATE, "--lib", "--"])
        .current_dir(project)
        .env_remove("CARGO_TARGET_DIR");
    if let Some(f) = filter {
        cmd.arg(f);
    }
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            return EditOutcome {
                rows: Vec::new(),
                seconds: 0.0,
                error: Some(format!("could not run cargo: {e}")),
            };
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows = parse_libtest(&stdout);
    let error = (rows.is_empty() && !out.status.success()).then(|| {
        let err = String::from_utf8_lossy(&out.stderr);
        err.lines()
            .filter(|l| l.contains("error"))
            .take(8)
            .collect::<Vec<_>>()
            .join("\n")
    });
    EditOutcome {
        rows,
        seconds: started.elapsed().as_secs_f32(),
        error,
    }
}

/// `test name ... ok|FAILED|ignored` lines, and each failure's captured output.
pub fn parse_libtest(stdout: &str) -> Vec<TestRow> {
    let mut rows: Vec<TestRow> = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("test ") else {
            continue;
        };
        let Some((name, result)) = rest.rsplit_once(" ... ") else {
            continue;
        };
        let status = match result.trim() {
            "ok" => TestStatus::Passed,
            "FAILED" => TestStatus::Failed,
            r if r.starts_with("ignored") => TestStatus::Ignored,
            _ => continue,
        };
        let mut row = TestRow::new(name.trim());
        row.status = status;
        rows.push(row);
    }
    // Failure output: "---- name stdout ----" up to the next block or the summary.
    let mut current: Option<(String, Vec<&str>)> = None;
    let mut blocks: Vec<(String, String)> = Vec::new();
    for line in stdout.lines() {
        if let Some(name) = line
            .strip_prefix("---- ")
            .and_then(|l| l.strip_suffix(" stdout ----"))
        {
            if let Some((n, l)) = current.take() {
                blocks.push((n, l.join("\n")));
            }
            current = Some((name.to_owned(), Vec::new()));
        } else if line == "failures:" || line.starts_with("test result:") {
            if let Some((n, l)) = current.take() {
                blocks.push((n, l.join("\n")));
            }
        } else if let Some((_, l)) = current.as_mut() {
            l.push(line);
        }
    }
    for (name, text) in blocks {
        if let Some(r) = rows.iter_mut().find(|r| r.name == name) {
            r.message = text.trim().to_owned();
        }
    }
    rows
}

impl crate::Editor {
    /// Advances a play-mode test run (called every editor frame).
    pub(crate) fn update_tests(&mut self, engine: &mut oxide_engine::Engine) {
        self.st.tests.poll_edit();
        let names: Vec<&'static str> = engine
            .world
            .try_resource::<oxide_scene::PlayModeTests>()
            .map(|t| t.names())
            .unwrap_or_default();
        self.st.tests.sync_play_tests(&names);
        let Some(run) = self.st.tests.play_run.as_mut() else {
            return;
        };
        // Play mode first (each test then gets a fresh scene inside it).
        if self.st.play == crate::PlayState::Edit {
            if run.entered_play {
                // Stopped from outside: give up.
                self.st.tests.play_run = None;
                self.st.tests.status = "Play-mode test run stopped".into();
                return;
            }
            run.entered_play = true;
            self.enter_play(engine);
            return;
        }
        let run = self.st.tests.play_run.as_mut().expect("checked");
        let w = &mut engine.world;
        if let Some((name, started, outcome)) = &run.current {
            let elapsed = started.elapsed().as_secs_f32();
            let result = outcome.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let result = match result {
                Some(r) => Some(r),
                None if elapsed > PLAY_TIMEOUT => {
                    Some(Err(format!("timed out after {PLAY_TIMEOUT:.0} s")))
                }
                None => None,
            };
            let Some(result) = result else {
                return;
            };
            let name = name.clone();
            if let Some(row) = self.st.tests.play.iter_mut().find(|r| r.name == name) {
                row.seconds = Some(elapsed);
                match result {
                    Ok(()) => row.status = TestStatus::Passed,
                    Err(m) => {
                        row.status = TestStatus::Failed;
                        row.message = m;
                    }
                }
            }
            let run = self.st.tests.play_run.as_mut().expect("running");
            run.current = None;
            if let Some(c) = w.try_resource_mut::<oxide_scene::Coroutines>() {
                c.clear();
            }
        }
        let run = self.st.tests.play_run.as_mut().expect("running");
        match run.queue.pop_front() {
            Some(name) => {
                // A fresh scene for every test (Unity's test scene).
                w.clear(false);
                let test = w
                    .try_resource::<oxide_scene::PlayModeTests>()
                    .and_then(|t| t.tests.iter().find(|t| t.name == name).cloned());
                match test {
                    Some(t) => {
                        let (_, outcome) = oxide_scene::testing::start_play_mode_test(w, &t);
                        run.current = Some((name, Instant::now(), outcome));
                    }
                    None => {
                        if let Some(row) = self.st.tests.play.iter_mut().find(|r| r.name == name) {
                            row.status = TestStatus::Failed;
                            row.message = "test not found (Scripts reloaded?)".into();
                        }
                    }
                }
            }
            None => {
                let entered = run.entered_play;
                self.st.tests.play_run = None;
                let failed = self
                    .st
                    .tests
                    .play
                    .iter()
                    .filter(|r| r.status == TestStatus::Failed)
                    .count();
                self.st.tests.status = format!(
                    "{} play-mode test(s), {failed} failed",
                    self.st.tests.play.len()
                );
                if entered && self.st.play != crate::PlayState::Edit {
                    self.exit_play(engine);
                }
            }
        }
    }
}

fn status_icon(s: TestStatus) -> RichText {
    match s {
        TestStatus::Passed => RichText::new("✔").color(Color32::from_rgb(110, 200, 110)),
        TestStatus::Failed => RichText::new("✖").color(Color32::from_rgb(230, 90, 80)),
        TestStatus::Running => RichText::new("…").color(Color32::from_rgb(220, 200, 90)),
        TestStatus::Ignored => RichText::new("–").color(Color32::GRAY),
        TestStatus::NotRun => RichText::new("○").color(Color32::GRAY),
    }
}

pub fn window_ui(
    st: &mut crate::EditorState,
    engine: &mut oxide_engine::Engine,
    ui: &mut egui::Ui,
) {
    let project = engine
        .world
        .resource::<oxide_assets::AssetServer>()
        .full_path("");
    let t = &mut st.tests;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut t.tab, 0, "PlayMode");
        ui.selectable_value(&mut t.tab, 1, "EditMode");
    });
    ui.separator();
    let running = t.running();
    ui.horizontal_wrapped(|ui| {
        let rows = if t.tab == 0 { &t.play } else { &t.edit };
        let failed: Vec<String> = rows
            .iter()
            .filter(|r| r.status == TestStatus::Failed)
            .map(|r| r.name.clone())
            .collect();
        let selected = t.selected.clone();
        if ui
            .add_enabled(!running, egui::Button::new("Run All"))
            .clicked()
        {
            if t.tab == 0 {
                t.run_play(None);
            } else {
                t.run_edit(project.clone(), None);
            }
        }
        if ui
            .add_enabled(
                !running && selected.is_some(),
                egui::Button::new("Run Selected"),
            )
            .clicked()
            && let Some(name) = selected
        {
            if t.tab == 0 {
                t.run_play(Some(vec![name]));
            } else {
                t.run_edit(project.clone(), Some(name));
            }
        }
        if ui
            .add_enabled(
                !running && !failed.is_empty(),
                egui::Button::new("Rerun Failed"),
            )
            .clicked()
        {
            if t.tab == 0 {
                t.run_play(Some(failed));
            } else {
                // libtest takes one filter: rerun all when several failed.
                let f = (failed.len() == 1).then(|| failed[0].clone());
                t.run_edit(project.clone(), f);
            }
        }
        if running {
            ui.spinner();
        }
    });
    if !t.status.is_empty() {
        ui.weak(&t.status);
    }
    ui.separator();
    let rows = if t.tab == 0 {
        t.play.clone()
    } else {
        t.edit.clone()
    };
    if rows.is_empty() {
        ui.weak(if t.tab == 0 {
            "No play-mode tests. Add `async fn my_test(co: Co)` functions to the Scripts and list \
             them in `scripts! { ...; tests: my_test }`."
        } else {
            "No edit-mode tests run yet. Write #[test] functions in the Scripts crate, then Run All."
        });
    }
    let (passed, failed) = (
        rows.iter()
            .filter(|r| r.status == TestStatus::Passed)
            .count(),
        rows.iter()
            .filter(|r| r.status == TestStatus::Failed)
            .count(),
    );
    if !rows.is_empty() {
        ui.weak(format!(
            "{} tests · {passed} passed · {failed} failed",
            rows.len()
        ));
    }
    egui::ScrollArea::vertical()
        .max_height((ui.available_height() - 120.0).max(80.0))
        .show(ui, |ui| {
            for r in &rows {
                ui.horizontal(|ui| {
                    ui.label(status_icon(r.status));
                    let sel = t.selected.as_deref() == Some(r.name.as_str());
                    if ui.selectable_label(sel, &r.name).clicked() {
                        t.selected = Some(r.name.clone());
                    }
                    if let Some(s) = r.seconds {
                        ui.weak(format!("{s:.2} s"));
                    }
                });
            }
        });
    ui.separator();
    if let Some(r) = t
        .selected
        .as_ref()
        .and_then(|n| rows.iter().find(|r| &r.name == n))
    {
        ui.label(RichText::new(&r.name).strong());
        ui.label(format!("{:?}", r.status));
        if !r.message.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("test-message")
                .show(ui, |ui| {
                    ui.monospace(&r.message);
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libtest_output_parses_into_rows() {
        let out = "\nrunning 3 tests\ntest tests::adds ... ok\ntest tests::breaks ... FAILED\ntest tests::slow ... ignored\n\nfailures:\n\n---- tests::breaks stdout ----\n\nthread 'tests::breaks' panicked at src/lib.rs:9:9:\nassertion `left == right` failed\n  left: 2\n right: 3\n\n\nfailures:\n    tests::breaks\n\ntest result: FAILED. 1 passed; 1 failed; 1 ignored\n";
        let rows = parse_libtest(out);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].status, TestStatus::Passed);
        assert_eq!(rows[1].status, TestStatus::Failed);
        assert!(rows[1].message.contains("left: 2"), "{}", rows[1].message);
        assert_eq!(rows[2].status, TestStatus::Ignored);
    }
}
