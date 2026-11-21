use super::*;

#[test]
fn file_picker_wait_cancels_only_its_owned_panel_and_finishes_without_a_callback() {
    let cancellation = Cancellation::default();
    let worker_cancel = cancellation.clone();
    let (_tx, rx) = crossbeam_channel::bounded::<Vec<PathBuf>>(1);
    let (closed_tx, closed_rx) = crossbeam_channel::bounded(1);
    let worker = std::thread::spawn(move || wait_for_selection(&rx, &worker_cancel, || closed_tx.send(()).unwrap()));
    cancellation.cancel();
    closed_rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert!(worker.join().unwrap().is_none());
    assert!(closed_rx.try_recv().is_err());
}

#[test]
fn file_picker_wait_keeps_a_completed_selection_and_closes_on_callback_failure() {
    let (tx, rx) = crossbeam_channel::bounded(1);
    tx.send(vec![PathBuf::from("selected.png")]).unwrap();
    assert_eq!(
        wait_for_selection(&rx, &Cancellation::default(), || panic!("already closed")),
        Some(vec![PathBuf::from("selected.png")])
    );
    drop(tx);
    let closed = std::cell::Cell::new(false);
    assert!(wait_for_selection(&rx, &Cancellation::default(), || closed.set(true)).is_none());
    assert!(closed.get());
}

#[test]
fn file_picker_cancel_before_start_skips_the_native_dialog() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let paths: Vec<PathBuf> = cancellation.pick(|| panic!("a cancelled picker must not open"));
    assert!(paths.is_empty());
}

#[test]
fn file_picker_panic_and_normal_cancel_both_finish_the_request() {
    for panic in [false, true] {
        let (tx, rx) = crossbeam_channel::unbounded();
        let request = Uuid::new_v4();
        spawn(request, tx, egui::Context::default(), move || {
            assert!(!panic, "injected picker failure");
            Vec::new()
        });
        assert!(
            matches!(rx.recv_timeout(std::time::Duration::from_secs(5)), Ok(Worker::Loaded(id, entries)) if id == request && entries.is_empty())
        );
    }
}

#[test]
fn file_picker_close_retires_only_its_owner_and_ignores_late_results_after_reopen() {
    let ctx = egui::Context::default();
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    for owner in [Choice::Bulk, Choice::Texture, Choice::Local] {
        let mut imports = Imports::default();
        let existing = Entry::new("existing.png".into(), Err("fixture".into()), Default::default());
        let existing_id = existing.id;
        imports.entries.push(existing);
        let request = Uuid::new_v4();
        let cancellation = Cancellation::default();
        imports.pending_files = Some(request);
        imports.pending_file_choice = Some(Choice::Texture);
        imports.pending_picker = Some(Pending {
            owner: Some(owner),
            cancellation: cancellation.clone(),
        });
        imports.cancel_file_picker(Choice::Sound);
        assert!(!cancellation.cancelled());
        assert_eq!(imports.pending_files, Some(request));

        imports.cancel_file_picker(owner);
        assert!(cancellation.cancelled());
        assert!(imports.pending_files.is_none());
        assert!(imports.pending_file_choice.is_none());
        assert_eq!(imports.entries[0].id, existing_id);

        let next_request = Uuid::new_v4();
        imports.pending_files = Some(next_request);
        imports
            .tx
            .send(Worker::Loaded(
                request,
                vec![Entry::new("late.png".into(), Err("fixture".into()), Default::default())],
            ))
            .unwrap();
        imports.poll(&ctx, &jobs, &world, &Default::default());
        assert_eq!(
            imports.pending_files,
            Some(next_request),
            "the old result must not unlock the new picker"
        );
        assert_eq!(imports.entries.len(), 1, "cancelled files must not join the retained drafts");
        imports.tx.send(Worker::Loaded(next_request, Vec::new())).unwrap();
        imports.poll(&ctx, &jobs, &world, &Default::default());
        assert!(imports.pending_files.is_none());
    }
}

#[test]
fn file_picker_from_the_texture_picker_is_not_owned_by_an_import_window() {
    let cancellation = Cancellation::default();
    let mut imports = Imports {
        pending_picker: Some(Pending {
            owner: None,
            cancellation: cancellation.clone(),
        }),
        ..Default::default()
    };
    imports.cancel_file_picker(Choice::Bulk);
    imports.cancel_file_picker(Choice::Local);
    assert!(!cancellation.cancelled());
    drop(imports);
    assert!(cancellation.cancelled(), "ending the session cancels its pending picker too");
}

#[cfg(windows)]
#[test]
#[ignore = "opens native Windows dialogs; run alone on an interactive desktop"]
fn file_picker_native_windows_close_after_the_dialog_is_visible() {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{EnumThreadWindows, IsWindowVisible},
    };
    unsafe extern "system" fn visible(window: HWND, data: LPARAM) -> i32 {
        // SAFETY: EnumThreadWindows supplies the HWND and our synchronous call
        // supplies a valid mutable bool pointer for the enumeration's lifetime.
        if unsafe { IsWindowVisible(window) } != 0 {
            unsafe { *(data as *mut bool) = true };
        }
        1
    }
    for kind in 0..3 {
        let cancellation = Cancellation::default();
        let worker_cancel = cancellation.clone();
        let (started_tx, started_rx) = crossbeam_channel::bounded(1);
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let worker = std::thread::spawn(move || {
            // SAFETY: no arguments; identifies only this test-owned thread.
            started_tx.send(unsafe { GetCurrentThreadId() }).unwrap();
            let result = worker_cancel.select(
                match kind {
                    0 => Kind::Files,
                    1 => Kind::Folder,
                    _ => Kind::File,
                },
                "Aurora import-picker-cancel [AUTOMATION - DO NOT TOUCH]",
                "Images",
                &["png"],
            );
            done_tx.send(result).unwrap();
        });
        let thread = started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut shown = false;
        while !shown && Instant::now() < deadline {
            // SAFETY: enumerate only this test's dedicated picker thread;
            // `shown` lives until the synchronous callback returns.
            unsafe { EnumThreadWindows(thread, Some(visible), &mut shown as *mut bool as LPARAM) };
            std::thread::sleep(Duration::from_millis(20));
        }
        cancellation.cancel();
        assert!(shown, "the test must exercise a visible native dialog");
        assert!(
            done_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("native dialog must return after cancellation")
                .is_empty()
        );
        worker.join().unwrap();
    }
}
