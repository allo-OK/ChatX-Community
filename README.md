<img src="work/tauri-client-lab/ui/chatx.png" alt="Logo ChatX" width="160">

# ChatX

ChatX adalah aplikasi chat kantor untuk Windows yang berjalan lewat jaringan lokal (LAN). **HOST** mengelola koneksi dan data bersama. **CLIENT** tersambung ke HOST untuk berkirim pesan dan bekerja bersama.

**Versi source saat ini** · 1.1.43 Andromeda

## Fitur

| Fitur | Keterangan |
| --- | --- |
| Chat dan lampiran | Chat pribadi dan grup, berbagi file, koleksi stiker, dan antrean kirim. |
| Tag dan notifikasi | Pilih akun dari daftar `@` dengan nama PC sebagai pembeda. Popup tampil satu per satu dan pesan biasa dari chat yang sama bisa diringkas. |
| File Gallery | Cari file dari folder yang didaftarkan. Indeks diperbarui di latar belakang setiap 30 menit selama tersambung ke HOST. |
| Pengelolaan HOST | Moderation, pengaturan akses perangkat, backup, dan distribusi update CLIENT lewat LAN. |

## Struktur repo

- [Aplikasi Windows](work/tauri-client-lab/src-tauri) · Backend Rust, integrasi Tauri, dan konfigurasi installer.
- [Antarmuka dan modul bersama](work/source-v3.7.11/ChatX) · Renderer yang dipakai oleh HOST dan CLIENT.
- [Skrip build](work/tauri-client-lab/build-community.ps1) · Membangun dan mengemas installer HOST serta CLIENT edisi publik.

## Build di Windows

Gunakan Rust dengan toolchain MSVC, Visual Studio Build Tools, dan Node.js. Dari root repo, jalankan:

```powershell
Set-Location work/source-v3.7.11/ChatX
npm ci
Set-Location ../../tauri-client-lab/src-tauri
cargo build --release --bins
```

Untuk membuat kedua installer, jalankan `../build-community.ps1 -NsisCompiler C:/path/to/makensis.exe` dari folder `src-tauri`. Hasilnya ada di `target/release/community`. Installer memakai folder edisi publik tersendiri.

## Kolaborasi

Buat branch untuk perubahan, uji alurnya di Windows, lalu buka pull request dengan ringkasan dan hasil tes. Simpan database chat, data profil pengguna, kredensial, kunci signing, dan hasil build di luar Git.

**Kredit dan kontributor** · Allo, Codex, Claude, Jrangbri
