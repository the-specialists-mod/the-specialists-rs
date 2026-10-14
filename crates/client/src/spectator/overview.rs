//! `CHudSpectator`'s overview map and player name tags. TS draws them only for HLTV spectators
//! (`IsSpectateOnly`), from the SDK 2.2-era code (decomp `unnamed_10021d70.c`):
//!
//! * `ParseOverviewFile` (`0x10023720`) reads `overviews/<map>.txt` after a director reset, and
//!   `LoadMapSprites` (`0x10023c60`) loads its image.
//! * `HUD_AddEntity` adds every player that passes its visibility tests to a 128-entry list
//!   (`AddOverviewEntity` `0x100249f0`, team-coloured icon, gone after this frame); a `DeathMsg`
//!   adds the victim's dead icon for 2 s (`0x10024a60`).
//! * `HUD_DrawNormalTriangles` → `DrawOverview` (`0x10024970`): the map image as tiles at the
//!   layer height (`DrawOverviewLayer` `0x10023c90`), a 128-unit icon per entity with a beam down
//!   to the layer under players, and each player's screen position for the name tags
//!   (`DrawOverviewEntities` `0x10024160`), then expired entries are dropped (`0x100249b0`). It
//!   draws in the main view, and in the inset only in inset mode 3.
//! * `CHudSpectator::Draw` (`0x10023080`) writes the names (`cl_drawnames`) under the icons,
//!   coloured by team, skipping those under the inset window.
//!
//! TS never sets an inset mode other than 0 (`specmode` and the director reset both pass 0) nor
//! the overview zoom/pan deltas (`+0xc98`/`+0xc9c`, only ever zeroed), so the inset-only parts
//! (camera icon, inset exclusion, the map view) are ported but unreachable, as in TS. A ruleset
//! with its own HLTV controls (TS 2.1, [`super::HltvHost`]) sets the inset and the map view.

use super::{Spectator, in_eye_position, with};
use crate::state;
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::ext::{spectator as spec, tri};
use goldsrc::sys::*;
use shared::ts::obs;

/// `MAX_OVERVIEW_ENTITIES`.
const MAX_ENTITIES: usize = 128;
/// Name-tag slots (`+0x96c`, 12 bytes each).
const MAX_TAGS: usize = 64;
/// The most overview image cells across (×4) and down (×3): 32 is a 128×96-tile image (12288
/// frames), far beyond any real overview.
const MAX_TILE_CELLS: i32 = 32;

/// The parsed `overviews/<map>.txt` (`m_OverviewData`, `+0x1c..+0x148`). TS keeps one layer.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct OverviewData {
    origin: Vec3,
    zoom: f32,
    layers: i32,
    height: f32,
    image: String,
    rotated: bool,
    /// Inset window in 640×480 units: x, y, width, height (`+0x138`, `+0x13c`, `+0x144`, `+0x140`).
    pub(super) inset: [i32; 4],
}

impl Default for OverviewData {
    fn default() -> Self {
        Self { origin: [0.0; 3], zoom: 1.0, layers: 0, height: 0.0, image: String::new(), rotated: false, inset: [4, 4, 240, 180] }
    }
}

#[derive(Clone, Copy, Default)]
struct Entity {
    sprite: HSPRITE,
    /// Entity index (TS keeps the `cl_entity_t*`); 0 = free slot.
    entity: i32,
    kill_time: f64,
}

/// The engine's model for the overview image (`+0xc90`).
#[derive(Clone, Copy)]
struct MapSprite(*const model_s);

impl Default for MapSprite {
    fn default() -> Self {
        Self(std::ptr::null())
    }
}

/// Overview sprites (`CHudSpectator::VidInit` `0x10022fe0`).
#[derive(Default, Clone, Copy)]
pub(super) struct Sprites {
    player: HSPRITE,
    blue: HSPRITE,
    red: HSPRITE,
    dead: HSPRITE,
    tile: HSPRITE,
    beam: HSPRITE,
    camera: HSPRITE,
}

#[derive(Default)]
pub(super) struct Overview {
    pub(super) data: OverviewData,
    /// Map view zoom and origin (`+0x94c`, `+0x950`) and angles (`+0x95c`).
    map_zoom: f32,
    map_origin: Vec3,
    pub(super) map_angles: Vec3,
    map_sprite: MapSprite,
    entities: Vec<Entity>,
    /// Screen x, y and a visible flag per player (`+0x96c`).
    tags: Vec<[f32; 3]>,
    pub(super) sprites: Sprites,
    /// Which view pass is being drawn (`+0x18`): 0 main, 1 inset.
    pub(super) draw_cycle: i32,
}

/// `VidInit`'s overview sprites.
pub fn load_sprites() -> Sprites {
    let e = engine();
    Sprites {
        player: e.spr_load("sprites/iplayer.spr"),
        blue: e.spr_load("sprites/iplayerblue.spr"),
        red: e.spr_load("sprites/iplayerred.spr"),
        dead: e.spr_load("sprites/iplayerdead.spr"),
        tile: e.spr_load("sprites/tile.spr"),
        beam: e.spr_load("sprites/laserbeam.spr"),
        camera: e.spr_load("sprites/camera.spr"),
    }
}

// ---- overview file ------------------------------------------------------------------------------

/// `COM_ParseFile` tokens: whitespace-separated words, `"quoted strings"`, `//` comments, and
/// `{ } ( ) ' ,` as tokens of their own.
fn tokens(text: &str) -> Vec<String> {
    const SINGLE: &[u8] = b"{})(',";
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while i < b.len() && b[i] <= b' ' {
            i += 1;
        }
        if i >= b.len() {
            return out;
        }
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        if b[i] == b'"' {
            i += 1;
            let s = i;
            while i < b.len() && b[i] != b'"' {
                i += 1;
            }
            out.push(String::from_utf8_lossy(&b[s..i]).into_owned());
            i += 1;
            continue;
        }
        if SINGLE.contains(&b[i]) {
            i += 1;
        } else {
            while i < b.len() && b[i] > b' ' && !SINGLE.contains(&b[i]) {
                i += 1;
            }
        }
        out.push(String::from_utf8_lossy(&b[start..i]).into_owned());
    }
}

/// C `atof`/`atoi` on a token (leading number, 0 when there is none).
pub(crate) fn atof(s: &str) -> f32 {
    let s = s.trim_start();
    let end = s
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0))
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    s[..end].parse().unwrap_or(0.0)
}

/// `ParseOverviewFile`'s parser: fills `data` (already reset to the defaults) and returns the
/// error TS prints, if any. On an error TS keeps what it parsed so far.
pub(super) fn parse_overview(text: &str, data: &mut OverviewData) -> Result<(), String> {
    let t = tokens(text);
    let mut it = t.iter().map(String::as_str);
    fn next<'a>(it: &mut impl Iterator<Item = &'a str>) -> &'a str {
        it.next().unwrap_or("")
    }
    while let Some(tok) = it.next() {
        if tok.eq_ignore_ascii_case("global") {
            if next(&mut it) != "{" {
                return Err("expected { ".into());
            }
            loop {
                let key = next(&mut it);
                if key.is_empty() || key == "}" {
                    break;
                }
                if key.eq_ignore_ascii_case("zoom") {
                    data.zoom = atof(next(&mut it));
                } else if key.eq_ignore_ascii_case("origin") {
                    for o in &mut data.origin {
                        *o = atof(next(&mut it));
                    }
                } else if key.eq_ignore_ascii_case("rotated") {
                    data.rotated = atof(next(&mut it)) as i32 != 0;
                } else if key.eq_ignore_ascii_case("inset") {
                    // x, y, width, height; `__ftol` truncates.
                    for i in 0..4 {
                        data.inset[i] = atof(next(&mut it)) as i32;
                    }
                } else {
                    return Err(format!("{key} unkown"));
                }
            }
        } else if tok.eq_ignore_ascii_case("layer") {
            if data.layers == 1 {
                return Err(" too many layers ".into());
            }
            if next(&mut it) != "{" {
                return Err("expected { ".into());
            }
            loop {
                let key = next(&mut it);
                if key.is_empty() || key == "}" {
                    break;
                }
                if key.eq_ignore_ascii_case("image") {
                    data.image = next(&mut it).to_owned();
                } else if key.eq_ignore_ascii_case("height") {
                    data.height = atof(next(&mut it));
                } else {
                    return Err(format!("{key} unkown"));
                }
            }
            data.layers += 1;
        }
    }
    Ok(())
}

impl Overview {
    /// The director-reset part of `DirectorMessage` (`0x10023380`): the entity list is cleared,
    /// `ParseOverviewFile` (`0x10023720`) and `LoadMapSprites` (`0x10023c60`) run.
    pub(super) fn reset(&mut self) {
        self.data = OverviewData::default();
        let level = r::level_name();
        let map = level.strip_prefix("maps/").unwrap_or(&level);
        let map = map.get(..map.len().saturating_sub(4)).unwrap_or("");
        let path = format!("overviews/{map}.txt");
        match r::load_file(&path) {
            None => engine().print(&format!("Couldn't open file {path}. Using default values for overiew mode.\n")),
            Some(bytes) => match parse_overview(&String::from_utf8_lossy(&bytes), &mut self.data) {
                Ok(()) => {
                    self.map_zoom = self.data.zoom;
                    self.map_origin = self.data.origin;
                }
                Err(e) => engine().print(&format!("Error parsing overview file {path}. ({e})\n")),
            },
        }
        self.map_sprite = MapSprite(if self.data.layers > 0 { spec::load_map_sprite(&self.data.image) } else { std::ptr::null() });
    }

    /// The director reset clears the entity list (`+0x148`, 0x200 dwords).
    pub(super) fn reset_entities(&mut self) {
        self.entities.clear();
    }

    /// `AddOverviewEntityToList` (`0x10024ab0`): the first free slot, if any.
    fn add(&mut self, sprite: HSPRITE, entity: i32, kill_time: f64) -> bool {
        if self.entities.len() < MAX_ENTITIES {
            self.entities.resize(MAX_ENTITIES, Entity::default());
        }
        let Some(slot) = self.entities.iter_mut().find(|e| e.entity == 0) else { return false };
        *slot = Entity { sprite, entity, kill_time };
        true
    }

    /// `CheckOverviewEntities` (`0x100249b0`).
    fn expire(&mut self, now: f64) {
        for e in &mut self.entities {
            if e.kill_time < now {
                *e = Entity::default();
            }
        }
    }

    /// The overview file's zoom and origin, for [`super::HltvHost::overview`].
    pub(super) fn overview(&self) -> (f32, Vec3) {
        (self.data.zoom, self.data.origin)
    }

    /// The map view's zoom, origin and angles, for [`super::HltvHost::map_view`].
    pub(super) fn map_view(&self) -> (f32, Vec3, Vec3) {
        (self.map_zoom, self.map_origin, self.map_angles)
    }

    /// The name tags (`+0x96c`), for [`super::HltvHost::name_tags`].
    pub(super) fn tags(&self) -> &[[f32; 3]] {
        &self.tags
    }

    pub(super) fn set_map_view(&mut self, zoom: f32, origin: Vec3) {
        self.map_zoom = zoom;
        self.map_origin = origin;
    }

    /// `GetMapPosition` (`0x100240b0`): the inset map camera, `4096 / zoom` back from the map
    /// origin along the map angles, its height scaled like the layer.
    fn map_position(&self) -> Vec3 {
        let z = (90.0 - self.map_angles[0]) / 90.0 * self.map_origin[2];
        let fwd = v::scale(v::normalize(v::angle_vectors(self.map_angles).0), -(4096.0 / self.map_zoom));
        v::add(fwd, [self.map_origin[0], self.map_origin[1], z])
    }
}

/// The inset pass's map view (`V_CalcSpectatorRefdef` with inset mode 3): the pitch follows the
/// mouse between 51.25° and 90°, and the camera sits over the map ([`Overview::map_position`]).
pub(super) fn map_view(s: &mut Spectator, cl_viewangles: Vec3) -> (Vec3, Vec3) {
    let ov = &mut s.overview;
    ov.map_angles = [cl_viewangles[0] * (1.0 / 90.0) * 38.75 + 51.25, cl_viewangles[1], cl_viewangles[2]];
    (ov.map_position(), ov.map_angles)
}

/// `AddOverviewEntity` (`0x100249f0`), from `HUD_AddEntity` while observing: a solid player gets
/// its team icon for this frame (`now - 1`, so the draw's expiry drops it right after).
pub fn add_entity(kind: i32, e: &cl_entity_t) {
    if kind != 1 || e.curstate.solid == 0 || state::with(|s| s.spec_mode) == 0 {
        return;
    }
    let team = crate::hud::with(|h| h.players.get(e.index as usize).map_or(0, |p| p.team));
    let now = engine().client_time() as f64;
    with(|s| {
        let sp = s.overview.sprites;
        let sprite = match team {
            1 => sp.blue,
            2 => sp.red,
            _ => sp.player,
        };
        s.overview.add(sprite, e.index, now - 1.0);
    });
}

/// `CHudSpectator::DeathMessage` (`0x10024a60`), from `DeathMsg`: the victim's dead icon for 2 s.
pub fn death(victim: i32) {
    if !engine().entity(victim).is_some_and(|e| e.player != 0) {
        return;
    }
    let now = engine().client_time() as f64;
    with(|s| {
        let dead = s.overview.sprites.dead;
        s.overview.add(dead, victim, now + 2.0);
    });
}

// ---- drawing ------------------------------------------------------------------------------------

/// One textured quad of the layer: corners with their texture coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tile {
    frame: i32,
    corners: [([f32; 2], Vec3); 4],
}

/// `DrawOverviewLayer`'s tiling: `x_tiles × y_tiles` quads covering `8192 / zoom` units across
/// (4:3), image frames in row order. Rotated maps run along x, the others along y with the
/// texture turned.
fn layer_tiles(data: &OverviewData, x_tiles: i32, y_tiles: i32, z: f32) -> Vec<Tile> {
    let (xs, ys, zoom) = (data.origin[0], data.origin[1], data.zoom);
    let aspect = 4.0 / 3.0;
    let mut out = Vec::new();
    let mut frame = 0;
    if data.rotated {
        let x_step = 8192.0 / zoom / x_tiles as f32;
        let y_step = -(8192.0 / (zoom * aspect)) / y_tiles as f32;
        let mut y = ys + 4096.0 / (zoom * aspect);
        for _ in 0..y_tiles {
            let mut x = xs - 4096.0 / zoom;
            for _ in 0..x_tiles {
                let c = [
                    ([0.0, 0.0], [x, y, z]),
                    ([1.0, 0.0], [x + x_step, y, z]),
                    ([1.0, 1.0], [x + x_step, y + y_step, z]),
                    ([0.0, 1.0], [x, y + y_step, z]),
                ];
                out.push(Tile { frame, corners: c });
                frame += 1;
                x += x_step;
            }
            y += y_step;
        }
    } else {
        let x_step = -(8192.0 / zoom) / x_tiles as f32;
        let y_step = -(8192.0 / (zoom * aspect)) / y_tiles as f32;
        let mut x = xs + 4096.0 / (zoom * aspect);
        for _ in 0..y_tiles {
            let mut y = ys + 4096.0 / zoom;
            for _ in 0..x_tiles {
                let c = [
                    ([0.0, 0.0], [x, y, z]),
                    ([0.0, 1.0], [x + x_step, y, z]),
                    ([1.0, 1.0], [x + x_step, y + y_step, z]),
                    ([1.0, 0.0], [x, y + y_step, z]),
                ];
                out.push(Tile { frame, corners: c });
                frame += 1;
                y += y_step;
            }
            x += x_step;
        }
    }
    out
}

/// Tiles across and down: the image's frames form a 4:3 grid of `sqrt(frames / 12)` cells (TS
/// truncates), 8 × 6 tiles of `sprites/tile.spr` without an image. The cell count is capped at
/// [`MAX_TILE_CELLS`] (a port guard: TS would build millions of quads every frame for a sprite
/// claiming billions of frames).
fn tile_counts(map_frames: Option<i32>) -> (i32, i32) {
    match map_frames {
        Some(n) => {
            let i = (((n / 12) as f32).sqrt() as i32).min(MAX_TILE_CELLS);
            (i * 4, i * 3)
        }
        None => (8, 6),
    }
}

/// `DrawOverview` (`0x10024970`), from `HUD_DrawNormalTriangles`.
pub fn draw() {
    if !spec::is_spectate_only() {
        return;
    }
    let (pip, cycle) = with(|s| (s.pip, s.overview.draw_cycle));
    // TS 3.0 draws in every main view; a ruleset may limit that (TS 2.1: its map main mode).
    let main_view = crate::gameplay::current().hltv_overview_in_main_view();
    if (cycle == 0 && !main_view) || (cycle == 1 && pip != 3) {
        return;
    }
    // TS moves the camera at the end of `DrawOverviewEntities`; nothing drawn before it reads
    // the camera, and the ruleset's host borrows this state, so it moves first.
    if pip != 0 {
        refresh_camera(pip);
    }
    with(|s| {
        draw_layer(&s.overview);
        draw_entities(s);
        s.overview.expire(engine().client_time() as f64);
    });
}

fn z_scale(ov: &Overview) -> f32 {
    (90.0 - ov.map_angles[0]) * (1.0 / 90.0)
}

/// `DrawOverviewLayer` (`0x10023c90`).
fn draw_layer(ov: &Overview) {
    let map = ov.map_sprite.0;
    let has_map = !map.is_null();
    let dummy = r::sprite_model(ov.sprites.tile);
    let (x_tiles, y_tiles) = tile_counts(has_map.then(|| spec::model_frames(map)));
    let z = z_scale(ov) * ov.data.height;
    tri::render_mode(kRenderTransTexture as i32);
    tri::cull_face(TRICULLSTYLE_TRI_NONE);
    tri::color4f(1.0, 1.0, 1.0, 1.0);
    for t in layer_tiles(&ov.data, x_tiles, y_tiles, z) {
        if has_map {
            tri::sprite_texture(map, t.frame);
        } else {
            tri::sprite_texture(dummy, 0);
        }
        tri::begin(TRI_QUADS as i32);
        for (uv, p) in t.corners {
            tri::tex_coord2f(uv[0], uv[1]);
            tri::vertex3fv(&p);
        }
        tri::end();
    }
}

/// `RGB_YELLOWISH` (255, 160, 0) as TS's floats.
const YELLOWISH: [f32; 3] = [1.0, 160.0 / 255.0, 0.0];

/// `DrawOverviewEntities` (`0x10024160`).
fn draw_entities(s: &mut Spectator) {
    let z_scale = z_scale(&s.overview);
    let z = s.overview.data.height * z_scale;
    tri::cull_face(TRICULLSTYLE_TRI_NONE);
    s.overview.tags.resize(MAX_TAGS, [0.0; 3]);
    for t in &mut s.overview.tags {
        t[2] = -1.0;
    }
    let e = engine();
    let screen = e.screen();
    let project =
        |p: Vec3| e.world_to_screen_ndc(p).map(|[x, y]| [(x + 1.0) * screen.width as f32 * 0.5, (1.0 - y) * screen.height as f32 * 0.5]);
    for i in 0..s.overview.entities.len() {
        let en = s.overview.entities[i];
        if en.sprite == 0 {
            continue;
        }
        let Some(ent) = e.entity(en.entity) else { continue };
        tri::sprite_texture(r::sprite_model(en.sprite), 0);
        tri::render_mode(kRenderTransTexture as i32);
        let (right, up, _) = v::angle_vectors(ent.angles);
        let mut origin = ent.origin;
        tri::begin(TRI_QUADS as i32);
        tri::color4f(1.0, 1.0, 1.0, 1.0);
        for (uv, su, sr) in [([1.0, 0.0], 64.0, 64.0), ([0.0, 0.0], 64.0, -64.0), ([0.0, 1.0], -64.0, -64.0), ([1.0, 1.0], -64.0, 64.0)] {
            let mut p = v::ma(v::ma(origin, su, up), sr, right);
            p[2] *= z_scale;
            tri::tex_coord2f(uv[0], uv[1]);
            tri::vertex3fv(&p);
        }
        tri::end();
        if ent.player == 0 {
            continue;
        }
        // The beam from the icon down to the layer.
        origin[2] *= z_scale;
        tri::render_mode(kRenderTransAdd as i32);
        tri::sprite_texture(r::sprite_model(s.overview.sprites.beam), 0);
        tri::color4f(YELLOWISH[0], YELLOWISH[1], YELLOWISH[2], 0.3);
        let [ox, oy, oz] = origin;
        for (a, b) in [((4.0, 4.0), (-4.0, -4.0)), ((-4.0, 4.0), (4.0, -4.0))] {
            tri::begin(TRI_QUADS as i32);
            for (uv, p) in [
                ([1.0, 0.0], [ox + a.0, oy + a.1, oz - z_scale]),
                ([0.0, 0.0], [ox + b.0, oy + b.1, oz - z_scale]),
                ([0.0, 1.0], [ox + b.0, oy + b.1, z]),
                ([1.0, 1.0], [ox + a.0, oy + a.1, z]),
            ] {
                tri::tex_coord2f(uv[0], uv[1]);
                tri::vertex3fv(&p);
            }
            tri::end();
        }
        // Where `Draw` writes the name: under the icon, as far below as a point 32 units off.
        let Some(sc) = project(origin) else { continue };
        let off = project([ox + 32.0, oy + 32.0, oz]).map_or([0.0; 2], |o| [o[0] - sc[0], o[1] - sc[1]]);
        if let Some(t) = s.overview.tags.get_mut((ent.index - 1) as usize) {
            *t = [sc[0], sc[1] + (off[0] * off[0] + off[1] * off[1]).sqrt(), 1.0];
        }
    }
    if s.pip == 0 {
        return;
    }
    // The camera icon (inset modes only, so never in TS 3.0), after [`refresh_camera`].
    let [x, y, cz] = s.saved_origin;
    tri::render_mode(kRenderTransAdd as i32);
    tri::sprite_texture(r::sprite_model(s.overview.sprites.camera), 0);
    tri::color4f(YELLOWISH[0], YELLOWISH[1], YELLOWISH[2], 1.0);
    let fwd = v::scale(v::angle_vectors(s.saved_angles).0, 512.0);
    let (right, left) = (rotate_yaw(fwd, 45.0), rotate_yaw(fwd, -45.0));
    tri::begin(TRI_TRIANGLES as i32);
    for (uv, p) in [
        ([0.0, 0.0], [x + right[0], y + right[1], (cz + right[2]) * z_scale]),
        ([0.0, 1.0], [x, y, cz * z_scale]),
        ([1.0, 1.0], [x + left[0], y + left[1], (cz + left[2]) * z_scale]),
    ] {
        tri::tex_coord2f(uv[0], uv[1]);
        tri::vertex3fv(&p);
    }
    tri::end();
}

/// The in-eye refresh of the camera icon (`0x10024160`): with the in-eye inset (2) or observer
/// mode, `V_GetInEyePos` of the locked target, unless the ruleset has its own
/// ([`ClientRules::hltv_overview_camera`](crate::gameplay::ClientRules::hltv_overview_camera)).
fn refresh_camera(pip: i32) {
    let (target, mut origin, mut angles) = with(|s| (s.locked_target, s.saved_origin, s.saved_angles));
    if !crate::gameplay::current().hltv_overview_camera(&super::EngineHltv, &mut origin, &mut angles)
        && (pip == 2 || state::with(|st| st.spec_mode) == obs::IN_EYE)
    {
        in_eye_position(target, &mut origin, &mut angles);
    }
    with(|s| {
        s.saved_origin = origin;
        s.saved_angles = angles;
    });
}

/// `AngleMatrix((0, yaw, 0))` + `VectorTransform`: `p` turned about z.
fn rotate_yaw(p: Vec3, yaw: f32) -> Vec3 {
    let (s, c) = yaw.to_radians().sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]]
}

/// Team colour of a name tag (`Draw`): blue team 1, red team 2, grey otherwise.
fn tag_color(team: i32) -> [f32; 3] {
    match team {
        1 => [0.4, 0.4, 1.0],
        2 => [1.0, 0.4, 0.4],
        _ => [0.8, 0.8, 0.8],
    }
}

/// Whether a tag at `p` lies inside the inset window (`Draw` skips those while an inset shows).
fn under_inset(inset: [i32; 4], p: [f32; 2], w: i32, h: i32) -> bool {
    let (sx, sy) = (w as f32 * (1.0 / 640.0), h as f32 * (1.0 / 480.0));
    let [ix, iy, iw, ih] = inset;
    // C `int` sums of the overview file's values: wrap rather than overflow.
    ix as f32 * sx < p[0] && iy as f32 * sy < p[1] && p[0] < ix.wrapping_add(iw) as f32 * sx && p[1] < iy.wrapping_add(ih) as f32 * sy
}

/// `CHudSpectator::Draw` (`0x10023080`): HLTV name tags.
pub fn draw_names() {
    if !spec::is_spectate_only() || crate::hud::cvars().drawnames.value() == 0.0 {
        return;
    }
    let e = engine();
    let screen = e.screen();
    let (tags, pip, inset) = with(|s| (s.overview.tags.clone(), s.pip, s.overview.data.inset));
    for (i, t) in tags.iter().enumerate() {
        if t[2] < 0.0 || (pip != 0 && under_inset(inset, [t[0], t[1]], screen.width, screen.height)) {
            continue;
        }
        let idx = i as i32 + 1;
        // 0x100232e0: `sprintf("%s", g_PlayerInfoList[i].name)`, the engine's raw bytes.
        let name = e.player_info(idx).name_bytes;
        let team = crate::hud::with(|h| h.players.get(idx as usize).map_or(0, |p| p.team));
        e.draw_console_bytes(tag_x(t[0], &name), t[1] as i32, Some(tag_color(team)), &name);
    }
}

/// A name tag's left edge (`0x10023308`): the tag's x less 3 per byte of the name (`strlen`),
/// truncated (`__ftol`).
fn tag_x(x: f32, name: &[u8]) -> i32 {
    (x - (name.len() * 3) as f32) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_overview_file() {
        let txt = "// overview\nglobal\n{\n\tZOOM\t2.5\n\tORIGIN\t100 -200 -50\n\tROTATED\t1\n\tINSET 10 20 300 200.9\n}\n\nlayer\n{\n\tIMAGE\t\"overviews/ts_x.bmp\"\n\tHEIGHT\t-48\n}\n";
        let mut d = OverviewData::default();
        assert_eq!(parse_overview(txt, &mut d), Ok(()));
        assert_eq!((d.zoom, d.origin, d.rotated, d.inset), (2.5, [100.0, -200.0, -50.0], true, [10, 20, 300, 200]));
        assert_eq!((d.layers, d.image.as_str(), d.height), (1, "overviews/ts_x.bmp", -48.0));

        let mut d = OverviewData::default();
        assert_eq!(parse_overview("layer { image a } layer { image b }", &mut d), Err(" too many layers ".into()));
        assert_eq!(d.image, "a", "keeps what it parsed");
        let mut d = OverviewData::default();
        assert_eq!(parse_overview("global { size 3 }", &mut d), Err("size unkown".into()));
        assert_eq!(parse_overview("global zoom", &mut OverviewData::default()), Err("expected { ".into()));
        assert_eq!(OverviewData::default().inset, [4, 4, 240, 180]);
    }

    #[test]
    fn tag_x_counts_raw_bytes() {
        // A name with bytes >= 0x80 (not valid UTF-8): 3 pixels per byte, as `strlen` counts.
        assert_eq!(tag_x(100.0, b"\xe9t\xe9"), 91);
        assert_eq!(tag_x(10.5, b"abcd"), -1, "truncated toward zero");
    }

    #[test]
    fn tokenizer() {
        assert_eq!(tokens("a{b}\"c d\"//x\n e,f"), ["a", "{", "b", "}", "c d", "e", ",", "f"]);
    }

    #[test]
    fn layer_tiling() {
        assert_eq!(tile_counts(None), (8, 6));
        assert_eq!(tile_counts(Some(48)), (8, 6));
        assert_eq!(tile_counts(Some(12)), (4, 3));
        let d = OverviewData { zoom: 1.0, ..Default::default() };
        let t = layer_tiles(&d, 8, 6, 5.0);
        assert_eq!(t.len(), 48);
        // Not rotated: x runs from +3072 down, y from +4096 down, frames in row order.
        assert_eq!(t[0].corners[0].1, [3072.0, 4096.0, 5.0]);
        assert_eq!(t[1].corners[0].1, [3072.0, 3072.0, 5.0]);
        assert_eq!(t[8].corners[0].1, [2048.0, 4096.0, 5.0]);
        assert_eq!(t[47].frame, 47);
        let r = layer_tiles(&OverviewData { rotated: true, ..d }, 8, 6, 0.0);
        assert_eq!(r[0].corners[0].1, [-4096.0, 3072.0, 0.0]);
        assert_eq!(r[0].corners[2].1, [-3072.0, 3072.0 - 1024.0, 0.0]);
    }

    #[test]
    fn entity_list_and_tags() {
        let mut ov = Overview::default();
        assert!(ov.add(5, 3, 9.0));
        assert!(ov.add(6, 4, 11.0));
        ov.expire(10.0);
        assert_eq!(ov.entities.iter().filter(|e| e.entity != 0).count(), 1);
        for i in 0..200 {
            ov.add(1, 1 + i % 32, 100.0);
        }
        assert_eq!(ov.entities.len(), MAX_ENTITIES);
        assert!(!ov.add(1, 1, 100.0), "full");
        assert_eq!(tag_color(1), [0.4, 0.4, 1.0]);
        assert_eq!(tag_color(2), [1.0, 0.4, 0.4]);
        assert!(under_inset([4, 4, 240, 180], [100.0, 100.0], 640, 480));
        assert!(!under_inset([4, 4, 240, 180], [300.0, 100.0], 640, 480));
        assert!((rotate_yaw([1.0, 0.0, 0.0], 90.0)[1] - 1.0).abs() < 1e-6);
    }

    /// `overviews/<map>.txt` with any content, sprite frame counts and inset values of any size.
    #[test]
    fn garbage_overview_files() {
        let mut x = 0x0bad_f00d_u32;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        const WORDS: [&str; 16] = [
            "global",
            "layer",
            "{",
            "}",
            "zoom",
            "origin",
            "inset",
            "image",
            "height",
            "rotated",
            "\"",
            "//",
            "-1e39",
            "é",
            "+",
            "2147483648",
        ];
        for n in 0..1500 {
            let text: String = if n % 2 == 0 {
                (0..n % 64).map(|_| WORDS[rnd() as usize % WORDS.len()]).collect::<Vec<_>>().join(if n % 4 == 0 { " " } else { "" })
            } else {
                String::from_utf8_lossy(&(0..n % 97).map(|_| rnd() as u8).collect::<Vec<_>>()).into_owned()
            };
            let mut d = OverviewData::default();
            let _ = parse_overview(&text, &mut d);
            atof(&text);
            for p in [[0.0, 0.0], [f32::NAN, 1e30]] {
                under_inset(d.inset, p, 640, 480);
            }
        }
        assert_eq!(atof("é1"), 0.0);
        assert_eq!(atof(" -12.5x"), -12.5);
        assert_eq!(tokens("\"unterminated"), ["unterminated"]);
        let mut d = OverviewData::default();
        assert_eq!(parse_overview("global { inset 2147483647 2147483647 2147483647 2147483647 }", &mut d), Ok(()));
        assert!(!under_inset(d.inset, [1.0, 1.0], 640, 480));
        for n in [i32::MIN, -1, 0, 11, 12, i32::MAX] {
            let (w, h) = tile_counts(Some(n));
            assert!((0..=MAX_TILE_CELLS * 4).contains(&w) && (0..=MAX_TILE_CELLS * 3).contains(&h));
        }
        assert_eq!(layer_tiles(&OverviewData { zoom: 0.0, ..Default::default() }, 4, 3, f32::NAN).len(), 12);
    }
}
