# Community localization review

This directory holds importable catalogs while their wording and layout are being reviewed. The Viewer embeds files from `crates/aurora-i18n/locales/`. It loads a file from this directory only when a user explicitly imports it.

Spanish (`es`) is the first catalog in review. It contains 15,044 active messages against upstream `3fe6a320`. Wording has been checked for 6,757 messages, including mesh detail and object permissions in Construction, keyboard shortcuts, voice, camera and media preferences. Confirmed groups of repeated labels share wording review. Link and shortcut text gets an additional check before offline interface testing. Download [`es.ftl`](es.ftl), keep that filename, select **Use a language file (.ftl)** in **Preferences → Language and updates**, and restart the Viewer. To return to the selected built-in language, clear the option and restart.

Review one language at a time against the English catalog and the [translator guide](../TRANSLATING.md). Follow the Legacy Viewer's terminology and use formal address in interface text. Keep command names, URLs, Fluent variables and selectors intact. At each release, use `catalog_delta` to find added and changed source text and remove obsolete keys. A missing or unusable translation uses the English message.

### Checking a batch against a new release

Save the English catalog from the revision the translations were based on. With Cargo available, run the batch wrapper from the new checkout.

```powershell
./docs/localization-review/Review-Catalogs.ps1 `
    -PreviousEnglish ./previous-en.ftl `
    -CatalogDirectory ./language-packs `
    -ReportDirectory ./review-reports
```

The wrapper builds Aurora's `catalog_delta` once and writes one text worklist per language. Each worklist shows missing entries, changed English and retired IDs. Pass `-CurrentEnglish ./current-en.ftl` to review a saved source catalog, such as a release candidate, from another checkout. Group repeated messages by source and meaning, then review each affected screen in the offline Viewer. Run the catalog checks after applying the edits.

Before an importable catalog is proposed for bundling, finish the wording review, check the relevant screens in the offline Viewer, and record the human validation and remaining issues in the pull request.

The first five community catalogs were brought into upstream Aurora in [#4315](https://github.com/Aurora-Viewer/Aurora-Viewer/pull/4315), building on [#4220](https://github.com/Aurora-Viewer/Aurora-Viewer/pull/4220). Both discussions retain the original contributor credits.
