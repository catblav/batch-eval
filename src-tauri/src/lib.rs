mod api;
mod dataset;
mod judge;
mod run_store;
mod runner;
mod storage;

#[cfg(test)]
mod integration_test;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(runner::RunRegistry::default())
        .invoke_handler(tauri::generate_handler![
            storage::load_settings,
            storage::save_settings,
            storage::load_dataset,
            storage::save_dataset,
            storage::read_text_file,
            storage::save_text_file,
            storage::append_text_file,
            run_store::load_last_run,
            run_store::clear_run,
            dataset::parse_dataset_file,
            dataset::parse_dataset_text,
            runner::start_run,
            runner::cancel_run,
            runner::test_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
