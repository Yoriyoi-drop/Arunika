# Panduan kontribusi agen — Arunika

## Aturan mutlak: dilarang membuat monolit

**Jangan pernah membangun, memperluas, atau memindahkan tanggung jawab menjadi
monolit.** Aturan ini berlaku untuk kode, crate, proses runtime, dan desain
deployment.

- Setiap crate memiliki satu tanggung jawab dan API publik yang sempit.
- Jangan menaruh parsing APK, DEX, framework Android, UI, sandbox, dan backend
  platform dalam satu crate atau satu modul besar.
- CLI hanya mengurai perintah, menyajikan hasil, dan mengorkestrasi use case;
  logika domain harus berada pada crate pemiliknya.
- Jangan membuat crate `core`, `common`, atau `utils` sebagai tempat
  penampungan tanggung jawab yang tidak terkait.
- Framework/domain tidak boleh mengimpor API platform. Kode platform berada
  di `host-linux`/`host-macos` di belakang kontrak host yang eksplisit.
- Pertahankan dependency satu arah; jangan menambah siklus antarkrate.
- Jika satu perubahan membuat batas tanggung jawab kabur, pecah desainnya
  sebelum implementasi. Jangan menyelesaikannya dengan modul raksasa,
  singleton, atau conditional platform yang tersebar.
- Jangan membuat crate kosong sebagai placeholder. Tambahkan crate saat ada
  tanggung jawab dan API yang benar-benar dapat diuji.

## Batas keselamatan dan klaim

- Perlakukan APK, arsip, resource, DEX, ELF, serta data IPC sebagai input
  tidak tepercaya.
- `info`, `compat`, dan `doctor` tidak boleh mengeksekusi kode APK.
- Jangan melewati validasi, sandbox, permission, atau error demi fallback
  yang tampak sukses.
- Laporkan API, ABI, layanan, atau backend yang belum didukung secara eksplisit.
- Jangan menyatakan dukungan Android, Linux, macOS, atau kompatibilitas aplikasi
  kecuali ada implementasi dan pengujian yang membuktikannya.
- Periksa lisensi serta syarat distribusi sebelum menambahkan komponen upstream.

## Pedoman implementasi

- Ikuti pembagian dan dependency yang dijelaskan dalam `DESIGN.md`.
- Sebelum mengubah kontrak lintas crate, jelaskan pemilik kontrak dan arah
  dependensinya dalam perubahan.
- Utamakan tipe dan error eksplisit; hindari `unwrap`/`expect` pada jalur input
  tidak tepercaya dan hindari kegagalan yang ditelan.
- Tambahkan tes terfokus untuk perilaku baru, terutama input malformed,
  kegagalan sandbox, dan keterbatasan kompatibilitas.
- Jangan memperkenalkan JIT, loader ELF, atau akses host luas sebelum model
  keamanan dan pengujian yang sesuai tersedia.
- Nama Arunika masih sementara; jangan menganggap domain, merek, atau
  identifier paket sudah tersedia.

## Validasi

Jalankan dari root workspace setelah perubahan Rust:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Perbarui dokumentasi desain bila batas crate, mode kompatibilitas, platform,
atau klaim dukungan berubah.

