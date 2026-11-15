use super::*;

#[test]
fn auto_sort_is_offered_on_one_folder_of_the_agent_but_not_the_library_or_an_item() {
    let agent = Uuid::from_u128(2);
    let mut inv = Inventory::default();
    crate::world::inventory::demo::seed(&mut inv, agent);
    let prefs = InventoryPreferences::default();
    let facts = Facts {
        agent,
        ..Default::default()
    };
    let st = InventoryUi::default();
    let offered = |id: u128, inv: &Inventory| {
        let catalog = super::super::context_catalog::build(inv, Uuid::from_u128(id), &st, &prefs, &facts);
        catalog.entries.iter().find(|e| e.id == "auto-sort").map(|e| e.enabled)
    };
    assert_eq!(offered(8000, &inv), Some(true));
    // an item has no entry, the Library's folders a disabled one
    assert_eq!(offered(8100, &inv), None);
    let library = inv.lib_root.as_u128();
    assert_eq!(offered(library, &inv), Some(false));
}

#[test]
fn a_failed_batch_stops_the_sort_and_keeps_its_message() {
    let mut st = InventoryUi::default();
    st.open_sorter(Uuid::from_u128(8000));
    st.sorter.job = Some(Job {
        queue: VecDeque::new(),
        base: "Sorted".into(),
        total: 400,
        done: 200,
        sending: 200,
    });
    st.sorter.submitted = true;
    st.finish_sorter(None);
    assert!(st.sorter.job.as_ref().is_some_and(|job| job.done == 400 && job.sending == 0));
    st.sorter.submitted = true;
    st.finish_sorter(Some("network down"));
    assert!(st.sorter.job.is_none());
    assert_eq!(st.sorter.status, "network down");
}
