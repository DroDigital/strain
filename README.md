# STRAIN

A third-person action game set in a quarantined city. A meteor brought a virus that rearranges
people into Lovecraftian things. You are an ordinary man the virus chose to live in instead of
consume: **compatible**. Soulslike melee, roguelike mutation builds, ARPG loot. No guns for you —
melee and magic only.

Everything is written by hand in Rust on top of a few low-level crates (wgpu, winit, gilrs, cpal,
glam, bytemuck, serde/ron, directories, fastrand, plus gltf and png for reading assets). The engine
— renderer, skinning, animation blending, ECS, physics, AI, mixer — is custom; the art and sound
combine CC0 (public domain) assets with procedural generation:

- **Characters, weapons, vehicles and street props**: Quaternius *Zombie Apocalypse Kit* (rigged,
  animated glTF).
- **The infected**: fourteen custom models (every archetype and boss) made in Blender on top of
  the kit's zombie rigs. Bone spikes, tentacles, tumours, chitin plates, maws, crystals and glowing
  sacs are modelled in and skinned to the original skeletons. The bodies are reshaped and
  repainted with veins, bruising, blood and ember cracks, and ambient occlusion is baked in.
  Affixes and mutations still add procedural growths on top.
- **Realism pass** (Blender, every human, dog and infected): heads, hands and feet re-proportioned
  toward adult anatomy and limbs slimmed (about their own joints, so every clip still fits), flat
  colour swatches replaced by photo materials (skin, fabric, denim, leather, hair, bone, flesh),
  one level of creased subdivision with interpolated skin weights, and Cycles-baked occlusion.
- **Surfaces**: ambientCG photo textures with their normal, roughness and occlusion maps, combined
  with procedural windows, signs, veins and glow.
- **Sound effects and ambience**: recorded CC0 packs from OpenGameArt, several variants per sound.
- **Music**, UI sounds, sirens and radio: synthesised in real time.

All third-party assets are listed in [`assets/CREDITS.md`](assets/CREDITS.md) and embedded into the
binary at compile time, so the game is still a single executable.

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
- **Bodies.** The dead go limp: a Verlet ragdoll built from each model's own skeleton takes the
  killing blow and tumbles against the street and walls. Every creature comes apart its own way —
  blades sever, clubs crush heads, blasts tear off several pieces — and a severed part flies as a
  piece of that creature's mesh. Wounds differ per creature: arterial spray, acid that pools and
  burns, embers, crystal shards that shatter, larvae spilling out, spores, black mist. A thing
  that loses a leg crawls after you; one that loses an arm hits softer. Blood soaks into bodies
  and spatters and pools on the ground.
- **Destruction.** Pallets, barrels, barriers, crates, couches, pipes and hydrants break into
  pieces of their own geometry under heavy blows, slams and blasts (brutes smash barricades too);
  loose clutter is kicked and thrown, burst hydrants jet water.
- **A living city.** Soldier squads with radio chatter, civilians hiding or fleeing, people turning
  in the street, helicopters, a day-night cycle, rain and spore storms, notes and graffiti,
  abandoned checkpoints, and short in-engine cutscenes for the opening, every boss and the ending.

## Assets

`python3 tools/fetch_assets.py` reproduces `assets/` from the original sources (needs Pillow and
ffmpeg); the processed files are committed, so building never needs the network. Add a source by
extending the tables at the top of the script. Textures come at 512 px with a packed material map
(`*_n.png`: normal, roughness, occlusion).

The infected (`assets/models/Strain_*.glb`) are built by `tools/blender/build_enemies.py`. The
recipes are in `tools/blender/recipes.py`. Run it with any Python that can `import bpy` (Blender's
own, or `pip install bpy`):

```bash
python -I tools/blender/build_enemies.py                                  # every model
python -I tools/blender/build_enemies.py Strain_Bulk --preview /tmp/prev  # one, with Cycles renders
```

The builder splices new geometry into the original GLB, so the skeleton and animation clips stay
byte-for-byte identical (clips are shared between models by bone name). New parts use materials
named `T:<Tex>`, which select one of the game's texture layers. A vertex colour alpha below 1
marks emission.

Then run the realism pass over the finished characters and infected (it marks each GLB, so a
second run is a no-op; rebuild a model with the enemy builder first to redo it):

```bash
python -I tools/blender/realism.py                 # every character and infected
python -I tools/blender/realism.py Strain_Bulk     # one
```

## Look

Grounded and photographic. The scene pass is physically based: GGX specular with height-correlated
Smith visibility, photo materials with normal and roughness maps (cotangent-frame normal mapping,
no stored tangents), soft Poisson-PCF sun shadows, a shadow-casting key point light, hemispheric
and reflective ambient, wrap-lit subsurface scattering for skin and flesh, wet streets with
mirror puddles, and blood that soaks into bodies as they are hurt. Textures are trilinear with
16x anisotropy at the window's native resolution.

The post pass adds screen-space ambient occlusion, then ray-marches **dense, heterogeneous
volumetric fog**: a mist that hugs the street, a base haze, and a smog layer that thickens with
altitude so upper floors and rooftops sink into it, broken into drifting banks by 3D noise and
cut by sun shafts through the shadow map (Henyey-Greenstein phase) and halos around lamps. Then
camera motion blur, depth of field focused on the player, bloom, ACES tonemapping, a muted grade,
film grain and a half-LSB dither against banding.

The camera is cinematic: damped follow with look-ahead and shoulder framing, a lens and distance
that react to sprinting, charging and fights, impact kicks with a little roll, handheld sway,
rotational shake, and a slow-motion kill cam on finishing blows. Hits land with short,
weight-scaled hit-stop, a directional flinch through the spine, blood spray, mist and gore;
there are no damage numbers or white hit flashes unless turned on in the settings.

Graphics settings: render resolution (native by default), shadow quality, fog quality, particle
density, V-sync, windowed / borderless / fullscreen, and damage numbers.

## Code map

| Module | Contents |
| --- | --- |
| `render/` | wgpu renderer, WGSL shaders, procedural textures and primitive meshes |
| `world/` | districts, building kit, chunk streaming, rift dungeons |
| `anim.rs` | skinned-character controller (clip selection, crossfades, attack sync, hit flinch, severed parts), procedural growths, IK, verlet chains |
| `ragdoll.rs` | Verlet ragdolls built from a model's skeleton |
| `render/model.rs` | glTF loading, skeletons, clip sampling and retargeting |
| `tools/blender/` | Blender builder and recipes for the custom infected, and the realism pass for every character |
| `assets.rs`, `build.rs` | embedded asset table, PNG and WAV decoding |
| `game/combat.rs` | action phases, melee resolution, damage pipeline, projectiles, zones |
| `game/gore.rs` | per-creature dismemberment, gibs, arterial bleeding, blood pools |
| `game/debris.rs` | loose clutter and breakable props that shatter into fragments |
| `game/camera.rs` | cinematic third-person camera |
| `game/ai.rs` | perception, steering, archetypes, affixes, soldiers, civilians, bosses |
| `game/spawn.rs` | procedural enemy assembly, spawning, drops |
| `game/magic.rs` | spells and Strain |
| `game/player.rs`, `game/progress.rs` | controller, stats, mutations, death/Husk, Burn Pits, rifts |
| `game/loot.rs` | items, rolls, uniques, grid inventory |
| `game/ui.rs` | HUD and menus |
| `game/life.rs`, `game/cutscene.rs` | weather, day-night, helicopters, ambience, cutscenes |
| `audio.rs` | hand-written mixer, recorded sound variants, ambience loops, synthesised UI/sirens, adaptive generative music |
| `ecs.rs`, `physics.rs` | entity-component store, collision, spatial hash |
| `data/*.ron` | weapons, spells, items, uniques, mutations, enemy parts, affixes, bosses, text |

Content lives in the RON tables under `data/`, embedded at compile time, so it grows without engine
changes. `cargo test` validates the tables and the content minimums.

## Automated play-throughs

`STRAIN_AUTOTEST=<scenario>` drives the real game with scripted input and saves screenshots to
`STRAIN_SHOT_DIR`: `tour` (districts, combat, menus, every rift, a boss, death and respawn),
`look` (every character model and archetype lined up), `zoo` (each custom infected and boss
close up), `bugs` (regression checks for fixed bugs),
`roam` (walks every district checking for stuck or embedded states), `duel` (a combat bot fights a
street pack and the first boss),
`gore` (ragdolls and dismemberment on every body type), `smash` (props shattering),
`spells` (all 18 spells), `stress` (every mutation and unique, every archetype at every stage),
`progress` (Husk reclaim and rising, vendor, stash, all bosses, ending, save and continue) and
`title` / `opening`. `STRAIN_PROFILE=1` prints CPU timings.

## Releases

Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yml`, which builds and publishes a
universal macOS app (arm64 + x86_64 merged with `lipo`), a Windows `.zip` with the `.exe`, and a
Linux `.tar.gz`. Every push runs tests and a release build on Linux, Windows and macOS.
