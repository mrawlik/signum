# signum

A small desktop app for stamping your signature, a date, checkmarks and short
text onto PDFs. The original file's bytes are kept; stamps are appended as an
incremental update.

## Usage

- Open a PDF with 🗁, by dropping it on the window, or with `signum file.pdf`.
- Set your signature once with ⚙ next to **Sign** (choose an `.svg`, or drop one
  on the window). It is remembered.
- Click a stamp button (**Sign**, **Date**, **X**, **✔**, or type into **Text**
  and press 🗛), then click on the page to place it. Fill in **Place** to get
  `Place, YYYY-MM-DD` instead of just the date.
- Drag a stamp to move it. Hover over it and use the mouse wheel or Up/Down to
  resize it, and Del or right-click to remove it.
- **Ctrl+S** saves in place; **Save copy** writes `name.signed.pdf`.

## Install on Debian / Ubuntu

1. Install a C toolchain and Rust (skip what you already have):

   ```sh
   sudo apt install build-essential curl
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Install `cargo-deb` once:

   ```sh
   cargo install cargo-deb --locked
   ```

3. Build the package and install it:

   ```sh
   git clone https://github.com/mrawlik/signum.git
   cd signum
   cargo deb
   sudo apt install ./target/debian/signum_*.deb
   ```

signum then shows up in your app launcher and under "Open with…" for PDFs.
To update, `git pull` and repeat step 3. To uninstall, run `sudo apt remove signum`.

File dialogs need `xdg-desktop-portal`, which most desktops already have.

## Run without installing

```sh
cargo run --release -- path/to/file.pdf
```
