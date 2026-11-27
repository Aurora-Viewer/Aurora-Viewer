use aurora_i18n::{install_user_catalog, language, select_with, text, tr, user_catalog_selected};

#[test]
fn australian_user_catalog_inherits_british_then_american_english() {
    let source = "language-name = Custom Australian English\n\
                  common-apply = Custom apply\n";

    let installed = install_user_catalog("en-AU", source).expect("a readable file installs");

    assert_eq!(installed.code, "en-AU");

    select_with("en", true, None);

    assert!(user_catalog_selected());
    assert_eq!(language(), "en-AU");
    assert_eq!(text("common-apply"), "Custom apply");
    assert_eq!(text("inventory-tab-favorites"), "Favourites");
    assert_eq!(text("common-cancel"), "Cancel");
    assert_eq!(
        tr!("landmark-star-in-favorites", name = "Home"),
        "In Favourites: Home\nClick to edit it"
    );
    assert_eq!(tr!("language-item-count", count = 2), "2 items");
}
