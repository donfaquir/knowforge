# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.5.1] - 2025-05-18

### Fixed

- Fixed ordered list bug: pressing Enter after "1. test" no longer inserts spurious "3." text into the new line
- Fixed production build issue where minification broke the filterTransaction class-name check (switched from `constructor.name` to `instanceof ReplaceStep`)
- Patched `@milkdown/preset-commonmark` splitListItemCommand to pass correct `itemAttrs` in ordered list context, eliminating the root-cause race condition between syncListOrderPlugin and Vue NodeView re-render
