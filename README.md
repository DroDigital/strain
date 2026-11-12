# STRAIN

A third-person action game set in a quarantined city. A meteor brought a virus that rearranges
people into Lovecraftian things. You are an ordinary man the virus chose to live in instead of
consume: **compatible**. Soulslike melee, roguelike mutation builds, ARPG loot. No guns for you —
melee and magic only.

Everything is written by hand in Rust on top of a few low-level crates (wgpu, winit, gilrs, cpal,
glam, bytemuck, serde/ron, directories, fastrand). There are **no asset files**: every mesh,
texture, animation, sound effect and piece of music is generated in code while the loading screen
runs.

## Running

```bash
cargo run --release
```

That is all on macOS and Windows. On Linux the audio and gamepad crates link against the system
ALSA and udev libraries, so install their development packages once (Debian/Ubuntu:
`sudo apt install libasound2-dev libudev-dev`; Fedora: `alsa-lib-devel systemd-devel`).

Saves and settings live in the platform's user data folder (for example
`~/.local/share/strain` on Linux, `~/Library/Application Support/com.Strain.Strain` on macOS,
`%APPDATA%\Strain\Strain\data` on Windows). Set `STRAIN_SAVE_DIR` to use another folder.

## Controls

Everything is rebindable in **Controls**, for keyboard/mouse and gamepad separately.

| Action | Keyboard / mouse | Gamepad |
| --- | --- | --- |
| Move / look | WASD / mouse | left / right stick |
| Light attack (chain) | left mouse | RB |
| Heavy attack (hold to charge) | F | RT |
| Block — tap just before a hit to **parry** | right mouse | LB |
| Dodge (tap) / sprint (hold) | Space | B |
| Interact | E | A |
| Heal | R | X |
| Cast spell / next spell | Q / X | LT / D-pad right |
| Lock on (flick the look input to switch) | Tab | right stick click |
| Inventory, mutations, map, character | I | Select |
| Pause | Esc | Start |

A parry staggers the attacker and opens it to a **Rupture**: press light attack next to it.
F3 shows the frame rate, F11 toggles borderless fullscreen, F12 saves a screenshot.

## How it plays

- **Health, Stamina, Strain.** Stamina pays for attacks, dodges, blocks and sprinting. Every
  attack has wind-up, active and recovery phases; enemies telegraph theirs. Poise and stagger
  work both ways. Hits land with hit-stop, directional reactions, sound and particles.
- **Hands.** Each hand is a slot: two one-handers, one-hander + shield, one-hander + catalyst,
  two catalysts, or one two-hander. Ten weapon classes with their own movesets (knife, baton,
  cleaver, pipe, fire axe, sledgehammer, riot shield, bone blade, tendril whip, catalyst).
- **Magic and Strain.** There is no mana. Casting needs a catalyst or a mutated hand and adds
  Strain. Higher Strain makes spells stronger and triggers random afflictions; full Strain means a
  brief loss of control and heavy self-damage. Strain only falls when you land melee hits.
  Three schools — Flesh, Rift, Cinder — with six spells each. Cinder burns every infected and,
  a little, you.
- **Mutations.** Kills drop biomass. Each level offers three of 84 mutations in four tiers. They
  change how you play (dodges leave spores, parries grow tendrils, heavies pull enemies in), every
  one visibly changes your body, some convert a hand permanently, and there are synergies and
  mutually exclusive branches.
- **Death and the Husk.** Gear, lit Burn Pits, district unlocks and boss kills persist. Level and
  mutations belong to the current life: dying leaves a Husk holding them. Reach it to take them
  back. Die first and that build is gone — the Husk rises as an enemy wearing it.
- **Burn Pits.** Resting heals, refills heals, allows fast travel, moves the front line and
  re-seeds every spawn in the city with new infection stages and affixes.
- **Enemies** are assembled from modular parts. Rarity is infection stage: Stage I Carrier,
  II Shifted (1–2 affixes), III Bloomed (3–4), IV Apex (named roaming minibosses). Eight
  archetypes (rusher, brute, leaper, spitter, crawler swarm, shielded, ambusher, summoner) and 24
  behaviour-changing affixes (Calcified, Splitting, Tethered, Mimic, Broadcasting, Gravid,
  Volatile, Phasing, Regenerating, Magnetised, …). They roam, react to sound and fight soldiers.
- **Loot.** Two hands, head, body, legs, two trinkets; six rarities from common to mythic; item
  level, rolled stats and affixes; 34 uniques with rule-changing effects; source pools (military,
  civilian, biological, rift); grid inventory, stash, vendor, comparison tooltips, loot beams.
- **The city.** Six districts streamed in chunks from a modular building kit (facades,
  shopfronts, alleys, enterable interiors, rooftops, subway entrances): the Held military hub,
  three Contested districts with live three-way fights, two Lost districts overgrown and bending.
  Districts gate by difficulty, not walls; a district's Burn Pits only take once the previous
  rift is closed.
- **Rifts.** Four meteor fragments open onto alien dimensions — the Gut, the Lattice, the Kiln and
  the Choir — each a procedurally arranged dungeon with its own rules and a fixed multi-phase
  boss. Close all four to open the crater and the final boss.
- **A living city.** Soldier squads with radio chatter, civilians hiding or fleeing, people turning
  in the street, helicopters, a day-night cycle, rain and spore storms, notes and graffiti,
  abandoned checkpoints, and short in-engine cutscenes for the opening, every boss and the ending.

## Look

Pseudo-retro with modern technique: low-poly geometry, 64 px nearest-filtered textures, vertex
snapping, subtle affine texture wobble, a fixed 480p internal resolution upscaled to the window,
ordered dithering and a reduced palette per district — combined with volumetric fog and light
shafts, a sun shadow map, a shadow-casting key point light, GPU-instanced particles, bloom,
screen-space distortion for rifts and magic, procedural animation (two-bone IK with planted feet,
verlet tendrils, spring secondary motion), hit-stop and camera shake.

Graphics settings: internal resolution, shadow quality, fog quality, particle density, V-sync and
windowed / borderless / fullscreen.

## Code map

| Module | Contents |
| --- | --- |
| `render/` | wgpu renderer, WGSL shaders, procedural textures and primitive meshes |
| `world/` | districts, building kit, chunk streaming, rift dungeons |
| `anim.rs` | modular body plans, IK, verlet chains, swing curves |
| `game/combat.rs` | action phases, melee resolution, damage pipeline, projectiles, zones |
| `game/ai.rs` | perception, steering, archetypes, affixes, soldiers, civilians, bosses |
| `game/spawn.rs` | procedural enemy assembly, spawning, drops |
| `game/magic.rs` | spells and Strain |
| `game/player.rs`, `game/progress.rs` | controller, stats, mutations, death/Husk, Burn Pits, rifts |
| `game/loot.rs` | items, rolls, uniques, grid inventory |
| `game/ui.rs` | HUD and menus |
| `game/life.rs`, `game/cutscene.rs` | weather, day-night, helicopters, ambience, cutscenes |
| `audio.rs` | hand-written mixer, synthesised sound bank, adaptive generative music |
| `ecs.rs`, `physics.rs` | entity-component store, collision, spatial hash |
| `data/*.ron` | weapons, spells, items, uniques, mutations, enemy parts, affixes, bosses, text |

Content lives in the RON tables under `data/`, embedded at compile time, so it grows without engine
changes. `cargo test` validates the tables and the content minimums.

## Automated play-throughs

`STRAIN_AUTOTEST=<scenario>` drives the real game with scripted input and saves screenshots to
`STRAIN_SHOT_DIR`: `tour` (districts, combat, menus, every rift, a boss, death and respawn),
`spells` (all 18 spells), `stress` (every mutation and unique, every archetype at every stage),
`progress` (Husk reclaim and rising, vendor, stash, all bosses, ending, save and continue) and
`title` / `opening`. `STRAIN_PROFILE=1` prints CPU timings.

## Releases

Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yml`, which builds and publishes a
universal macOS app (arm64 + x86_64 merged with `lipo`), a Windows `.zip` with the `.exe`, and a
Linux `.tar.gz`. Every push runs tests and a release build on Linux, Windows and macOS.
