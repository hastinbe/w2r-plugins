# w2r-plugins

Plugins for **Warcraft II Remastered**, installed and updated with the **[W2R Mod Loader](https://github.com/hastinbe/w2r-mod-loader/releases)**.

Each plugin is a small add-on that changes or adds one thing: a chat history, a map preview during a
match, colored player boxes, a random-map button, and so on. This repository holds their source,
builds them in CI, and publishes the list the Mod Loader reads.

> A fan project. It is not made or endorsed by Blizzard. Plugins run inside the game, so install
> only ones you trust, and read what each one asks for before you accept.

## Install

You do not download anything from this repository by hand.

1. Download the **[W2R Mod Loader](https://github.com/hastinbe/w2r-mod-loader/releases)** and run it. On first start it offers to install the mod into your game, then to
   install plugins.
2. Open the **Browse** tab, pick plugins, and read the permission list in the window that opens.
3. Start the game from Battle.net as usual.

The Mod Loader checks every download against a size and SHA-256 listed in the index before it
installs it, and tells you when an update is available.

## The plugins

| Plugin | What it does | Needs | Everyone in the game? |
|---|---|---|---|
| **Chat** | In-match chat history with channels and player colors | game access, hooks | no |
| **Map Preview** | The lobby's map preview over a running match (backslash key) | game access, hooks | no |
| **Player Color Bar** | One box per player in the top bar, with team dividers | game access, hooks | no |
| **Random Map** | A Random map button on the Create screen, with a player-count filter and folder | nothing | no |
| **Remember Game Settings** | Fills in the settings of your last created game | game access, hooks | no |
| **Unit Color** | Recolors the eight team colors on your screen only, with profiles; Allied Minimap Colors (replaces Black Recolor) | game access, hooks | no |
| **Countdown Sound** | A beep for each second of the lobby countdown, audible in the background | game access, hooks | no |
| **Observer** | Pick OBS as your team to watch a game instead of playing | game access, hooks | **yes** |

"Hooks" means the plugin patches code inside the game, which the Mod Loader flags as dangerous at
install. It is how most of them draw on the game's own screens. Each plugin's `README.md` in
[plugins/](plugins/) lists exactly what it asks for and why.

Two folders, `w2r-lobby-log` and `w2r-sdk-probe`, are research tools for the developers. They are
never published.

## How it fits together

```
   plugins/<id>/ source ──PR──► main ──CI──► packages + index.json ──► GitHub Pages
                                                                                      │
                                              W2R Mod Loader ◄────────────────────────┘
                                              (downloads, verifies, installs)
```

- **This repository** holds the plugin source and the `w2r-publish` tool that checks manifests,
  packages plugins and builds the index.
- **CI** builds every plugin as a 32-bit DLL (the game is 32-bit), checks it, and on `main` publishes
  one package per plugin version plus the updated `index.json`. A published version never changes.
- **The Mod Loader app and the in-game host** live in the [w2r-mod-loader](https://github.com/hastinbe/w2r-mod-loader) repository. The host loads plugins through a
  small C interface; capabilities such as game access or chat are shown to the player and enforced
  as far as that is possible. They are a consent prompt, not a sandbox.

The full pipeline, including how the index is trusted, is in [docs/PIPELINE.md](docs/PIPELINE.md).

## Write a plugin

Anyone can. The process is a pull request from a feature branch, one plugin at a time.

- [CONTRIBUTING.md](CONTRIBUTING.md): the workflow and the rules (what needs extra review, versions,
  local checks).
- [docs/PLUGIN-GUIDE.md](docs/PLUGIN-GUIDE.md): how a plugin is built, with a skeleton, the ABI rules
  that matter, testing and common mistakes.
- [AGENTS.md](AGENTS.md): the same for AI coding agents, as a short list of rules. If you build with an
  assistant, start here.

Quick start:

```bash
rustup target add i686-pc-windows-msvc
cargo run -p w2r-publish -- check                                   # manifests
cargo test -p w2r-random-map                                        # a plugin's tests
cargo build --release --target i686-pc-windows-msvc -p w2r-random-map
```

## Repository layout

| Path | What is there |
|---|---|
| `plugins/` | one folder per plugin: `plugin.json`, `Cargo.toml`, `src/`, `README.md` |
| `tools/w2r-publish/` | checks manifests, builds packages and the index |
| `docs/` | the publishing pipeline and the plugin guide |
| `.github/` | CI and release workflows, PR template, code owners |

## Credits

Chat and Unit Color (which grew out of Black Recolor) are ports of features from **wc2r-plus** by Luxray997 (MIT); they keep the
original credit and license notice.
