# Community localization review

The Viewer embeds the catalogs in [`crates/aurora-i18n/locales/`](../../crates/aurora-i18n/locales/). Spanish, Italian, Polish, Turkish, European Portuguese, Norwegian Bokmål and Ukrainian became built-in languages in [#4374](https://github.com/Aurora-Viewer/Aurora-Viewer/pull/4374).

[Draft #4335](https://github.com/Aurora-Viewer/Aurora-Viewer/pull/4335) tracks the remaining language previews and provides download links. To test an import, leave its locale filename intact, select **Use a language file (.ftl)** in **Preferences → Language and updates**, and restart the Viewer. Clear that option and restart to return to a built-in language.

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

The first five community catalogs were brought into upstream Aurora in [#4315](https://github.com/Aurora-Viewer/Aurora-Viewer/pull/4315), building on [#4220](https://github.com/Aurora-Viewer/Aurora-Viewer/pull/4220). The seven later catalogs build on four drafts from #4220 and the [Italian contribution in #4259](https://github.com/Aurora-Viewer/Aurora-Viewer/issues/4259).
