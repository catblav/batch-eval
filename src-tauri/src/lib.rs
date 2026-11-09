mod api;
mod dataset;
mod judge;
mod report;
mod run_store;
mod runner;
mod script_pack;
mod storage;

#[cfg(test)]
mod integration_test;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(runner::RunRegistry::default())
        .manage(runner::GateRegistry::default())
        .setup(|app| {
            let h = app.handle().clone();
            // identifier 变更后，把旧配置目录的数据复制到新目录
            storage::migrate_legacy_config_dir(&h);
            // 旧版 runs/current + runs/history 迁移成扁平任务目录。
            // 迁移失败只保留旧目录，不影响启动。
            run_store::migrate_legacy_dirs(&h);
            // 进程被杀时残留的 running 任务标记为 interrupted，保留已完成结果
            run_store::recover_running(&run_store::runs_root(&h));
            run_store::cleanup_finished(&run_store::runs_root(&h));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            storage::load_settings,
            storage::save_settings,
            storage::load_dataset,
            storage::save_dataset,
            storage::read_text_file,
            storage::save_text_file,
            storage::append_text_file,
            storage::list_datasets,
            storage::import_dataset_file,
            storage::import_dataset_text,
            storage::preview_dataset,
            storage::get_dataset_row,
            storage::replace_dataset,
            storage::rename_dataset,
            storage::delete_dataset,
            storage::sync_dataset_mirror_cmd,
            run_store::list_runs,
            run_store::load_run,
            run_store::delete_run,
            dataset::parse_dataset_file,
            dataset::parse_dataset_text,
            report::export_excel_report,
            runner::start_run,
            runner::cancel_run,
            runner::probe_api,
            runner::test_connection,
            script_pack::auto_iterate_paths,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
