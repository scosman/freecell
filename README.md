<div align="center">
<img width="96" height="96" alt="sourceMacOS-1024x1024@2x" src="https://github.com/user-attachments/assets/9a9f7d98-7155-47df-98d2-67daec184ee1" />

# FreeCell
### The open spreadsheet app
</div>

- Free and OSS desktop spreadsheet app
- Supports XLSX format and 90% of Excel formulas
- GPU rendering: bar graphs at 240fps
- Insanely fast and light: Rust-based, 11MB app, launches in 60 milliseconds
- Cross platform: Mac, Windows and Linux

<div align="center">
  
[![Download for macOS](https://img.shields.io/badge/Download_for_macOS-1d1d1f?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/scosman/FreeCell/releases) [![Download for macOS](https://img.shields.io/badge/Download_for_Windows-1d1d1f?style=for-the-badge&logo=pcgamingwiki&logoColor=white)](https://github.com/scosman/FreeCell/releases) [![Download for macOS](https://img.shields.io/badge/Download_for_Linux-1d1d1f?style=for-the-badge&logo=linux&logoColor=white)](https://github.com/scosman/FreeCell/releases)
</div>

### Features

It’s a spreadsheet. It has the most of the features you’d come to expect including:

- Formulas: supports 90% of Excel formulas
- Formatting: all the text formatting, borders, fills and sizing you expect
- XLSX file support: open and edit Excel files (open OOXML format)
- Speed: it’s ridiculously fast. Sheets that take 30s to open in Apple Numbers open in <1s in FreeCell.
- 100% local software: no cloud, no analytics, completely private
- Cross platform: works on Mac, Windows and Linux
- Native: compiled desktop app, not Electron
- Charts: GPU rendered charting

What’s not included (yet):
- Pivot tables
- Merged cells
- Dynamic arrays (UNIQUE/FILTER/SORT)
- Clippy

### FAQ

**Why?** Honestly, to see if I could. 

I’ve been building more and more software with agentic engineering. This project came out of the question: can I recreate an app that hundreds of people have been working on for decades, with decent quality, in a short amount of time.

**How is it built** FreeCell is built in Rust. It’s an agentic engineering project (vibe coding but with tests), using the [vibe crafting skill](https://github.com/scosman/vibe-crafting).

**What engine does it use** Its core spreadsheet engine is [IronCalc](https://www.ironcalc.com). It’s an excellent Rust-based, Excel compatible spreadsheet framework.

**What rendering system does it use** It uses the [GPUI](https://gpui.rs) library, the same library behind the Zed editor. Plus [GPUI-component](https://github.com/longbridge/gpui-component).

**Why GPU rendering?** I enjoy the speed of apps like [Zed](https://zed.dev) and [Ghostty](https://ghostty.org). A spreadsheet is largely custom UI components with minimal reuse of system controls, so it’s a good fit for GPU rendering. The result: it’s buttery smooth even on the largest sheets and works on all major platforms. 

### Running from Source

```sh
cd App
cargo run -p freecell-app
```

### Building from Source

```sh
cd App
./scripts/package.sh
```

### License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE) 
- [MIT license](LICENSE-MIT) 

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
