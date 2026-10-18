//! `oxide-editor --project DIR`: the project editor, prebuilt in a release. The project's
//! Scripts compile against the same engine library and are loaded into it.

fn main() -> std::process::ExitCode {
    oxide_dylib::editor::launch::run_project_editor(oxide_dylib::loaded_build_id())
}
