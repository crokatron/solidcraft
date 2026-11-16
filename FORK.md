# crokatron development fork

This is an unofficial development fork of [tangency-labs/solidcraft](https://github.com/tangency-labs/solidcraft). It is not an official release and does not imply upstream endorsement or support.

- [Fork changelog](FORK_CHANGELOG.md): changes and known limitations.
- [Feature requests](FEATURE_REQUESTS.md): priorities and acceptance criteria.
- [Permission request draft](docs/distribution-permission-draft.md): unsent request concerning distribution of modified application builds.

## History and contributions

The customization starts from upstream commit `0c529228` (Release 0.40.1). Keep upstream ancestry and licence notices intact. Use `upstream` for the original repository and `origin` for the crokatron fork. Keep upstream `main` available and develop customizations on `crokatron/development`; use focused branches and pull requests for subsequent work. Do not force-push shared history.

Report fork-specific bugs and feature requests to this fork, with reproduction steps, expected and actual behaviour, and a small public-safe example where possible. Existing issue templates are available. Never attach private scans, credentials, customer designs or personal settings. Upstream contributions should be separated into reviewable changes following CONTRIBUTING.md.

## Validation and distribution

This branch is experimental. A feature appearing in source or the changelog is not a guarantee of production readiness. Record check results and known failures before publishing releases; do not present targeted tests as a full CI pass.

No modified binary release is published as part of creating this development fork. The engine and interface have different licence terms: retain LICENSE-MIT, LICENSE-APACHE, LICENSE-BRAND, NOTICE and applicable third-party notices. Distribution of a modified branded application is subject to LICENSE-BRAND; the permission request remains a draft until explicitly sent and answered.

Build output, recovery copies, user shortcut files and private project data do not belong in source control. Future distributable packages should use isolated settings, identify their source revision, include checksums and licence notices, and avoid silently updating from the upstream release feed.
