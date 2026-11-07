# TODO — Pengembangan menuju 1.0 Stabil LTS Production

Status: Fase 0 stabil (inspeksi APK). Fase 1 proof of concept (subset interpreter DEX) stabil. Semua tes lolos via `cargo nextest run`, fmt bersih, clippy bersih.

## Fase 1: Stabilisasi Interpreter DEX [SELESAI]

- [x] Perbaiki opcode 0x73 (invoke-virtual/range) — helper `invoke_virtual_range` di test-fixtures menggunakan opcode 0x73 (sebelumnya 0x74 / invoke-super/range)
- [x] Tambah tes stabilitas interpreter (`crates/dex-engine/tests/stability.rs`)
- [x] Tambah tes parser DEX hardening (`crates/dex-engine/tests/hardening.rs`)
- [x] Tambah tes integrasi `arunika-sandbox-manager` (kebijakan, ConfinedRoot, PermissionPolicy, MemoryAssetSource, asset_children)
- [x] Tambah tes integrasi `arunika-resource-manager` (tabel resource, ID sintetis, duplicate, nama tidak valid)
- [x] Tambah tes integrasi `arunika-activity-runtime` (lifecycle, intent, TaskStack)
- [x] Verifikasi semua tes lolos (439 tes), fmt bersih, clippy bersih
- [x] Dokumentasikan: TODO.md, CHANGELOG.md
- [x] Commit: `e15fd8a` fase 1 stabilisasi interpreter DEX dan uji regresi

## Fase 2: Hardening Domain Crates ke Produksi [SELESAI]

- [x] Tes integrasi publik untuk sandbox-manager, resource-manager, activity-runtime
- [x] Semua crate domain memiliki tipe error eksplisit (thiserror), tanpa unwrap/expect di jalur produksi
- [x] Semua crate mengikuti aturan dependensi satu arah, tanpa monolit
- [x] Verifikasi: cargo fmt, cargo clippy -D warnings, cargo nextest run

## Fase 3: MVP Fitur (rencana)

- [ ] Resource: integrasi tabel resource dengan AssetSource host-linux
- [ ] Activity: lifecycle penuh, intent resolusi, task stack
- [ ] Storage privat: AppDataFileSystem di host-linux mengimplementasi ConfinedFileSystem
- [ ] Clipboard: service-broker terhubung ke clipboard host
- [ ] Audio: audio-manager terhubung ke backend host (PipeWire/CoreAudio)
- [ ] Crash reporting: crash-reporter menangkap panic dan eksepsi tidak tertangkap

## Fase 4: Production (rencana)

- [ ] Sandbox diaudit: ConfinedFileSystem ditegakkan, tidak ada fallback diam-diam
- [ ] Regresi otomatis: CI dengan cargo nextest, fuzz parser
- [ ] Crash isolation: aplikasi crash tidak jatuhkan host
- [ ] Profiling: cold/warm start, RAM, waktu rendering
- [ ] Fallback Linux: backend jendela (Wayland/X11), audio (PipeWire)

## Validasi (dari root workspace)

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run
```
