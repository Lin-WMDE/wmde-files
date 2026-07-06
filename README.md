# WMDE Files

File manager for the WMDE desktop (COSMIC-based).

WMDE Files is part of the [WMDE desktop](https://wmde.fun) and is a fork of
[pop-os/cosmic-files](https://github.com/pop-os/cosmic-files) by System76, rebranded for WMDE.
Original authorship and copyright are retained.

## Build (Arch Linux)

A `PKGBUILD` is provided:

```sh
makepkg -si
```

Or build from source:

```sh
git clone https://github.com/Lin-WMDE/wmde-files
cd wmde-files
cargo build --release
```

## Credits

Based on [cosmic-files](https://github.com/pop-os/cosmic-files) by **System76** (Pop!_OS).
The COSMIC desktop environment is maintained by System76; a list of all COSMIC projects is in the
[cosmic-epoch](https://github.com/pop-os/cosmic-epoch) project. All original authorship and
copyright are retained; see [LICENSE](LICENSE).

## License

Licensed under [GPL-3.0-only](LICENSE).
