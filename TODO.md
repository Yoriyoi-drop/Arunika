# TODO — Fase 1: Stabilitas DEX Interpreter (Arunika)

Status: Fase 0 stabil (inspeksi). Fase 1 proof of concept (subset interpreter DEX). Semua tes lolos, fmt bersih, clippy bersih.

Tujuan: stabilkan Fase 1 dengan tes fokus, lalu maju menuju 1.0 produksi siap per fase.

## Tugas

- [x] Perbaiki opcode 0x73 (invoke-virtual/range) di `dex-engine/src/interpreter.rs` — opcode sudah dihandle; bug di `test-fixtures` (`invoke_virtual_range` pakai opcode 0x74 alih-alih 0x73) diperbaiki
- [x] Tambah tes stabilitas interpreter (`crates/dex-engine/tests/stability.rs`: wide values, null refs, array bounds, MAX_ARRAY_LEN, class initialization <clinit>, step limits, exception try/catch)
- [x] Tambah tes parser DEX hardening (`crates/dex-engine/tests/hardening.rs`: code_item/class_def/proto_ids offset luar berkas, mutasi header)
- [x] Verifikasi semua tes lolos, fmt bersih, clippy bersih (414 tests via `cargo nextest run`)
- [x] Dokumentasikan status stabilitas Fase 1 (CHANGELOG.md, TODO.md)

## Validasi (dari root workspace)

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run
```
