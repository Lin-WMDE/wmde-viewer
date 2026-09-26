# WMDE Viewer

Image viewer and markup tool for the WMDE desktop (COSMIC-based).

WMDE Viewer is part of the [WMDE desktop](https://wmde.fun) and is a fork of
[pop-os/cosmic-viewer](https://github.com/pop-os/cosmic-viewer) by Bryan Hyland and System76,
rebranded for WMDE. Original authorship and copyright are retained.

## Build (Arch Linux)

A `PKGBUILD` is provided:

```sh
makepkg -si
```

Or build from source:

```sh
git clone https://github.com/Lin-WMDE/wmde-viewer
cd wmde-viewer
just build-release
```

The build links the system `libheif` and `libjpeg-turbo` and expects the WMDE `libcosmic` fork
checked out next to this directory (`../libcosmic`).

## Credits

Based on [cosmic-viewer](https://github.com/pop-os/cosmic-viewer) by **Bryan Hyland** and
**System76** (Pop!_OS). The COSMIC desktop environment is maintained by System76; a list of all
COSMIC projects is in the [cosmic-epoch](https://github.com/pop-os/cosmic-epoch) project. All
original authorship and copyright are retained; see [LICENSE](LICENSE).

## License

Licensed under [GPL-3.0-only](LICENSE).
