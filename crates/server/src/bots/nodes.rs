//! TS's bot node graph, `CNodeGraph` (the global `g_NodeGraph` @ 0x100e8330 in mp.dll): the
//! learned waypoints, their links, the `.ndm` files, and `FindPath`. See the decomp's
//! `docs/server/bots.md` §11.1–11.6.
//!
//! Players (humans and bots) drop a node every 0.1 s while `botlearn > 0` ([`learn`]); each new
//! node is linked to every existing node it can see and walk to. `FindPath` is a Dijkstra from
//! the start node followed by a greedy walk back from the goal, with TS's quirks kept (the
//! backtrack pops the current node, ties go to the lowest index, the 1000-iteration limits).
//!
//! The graph talks to the engine only through [`NodeWorld`], so it's unit-tested with a fake
//! world and against the decomp's replica (`scripts/ndm.py`) on the shipped `.ndm` files.
//!
//! Floating point follows mp.dll's x87 code: values TS stores to a `float` are `f32` here, and
//! distances that stay on the x87 stack until `__ftol` (sqrt included) are computed in `f64`,
//! MSVC's default 53-bit precision. The 199,704 link distances in the shipped files all equal
//! the `f64` value truncated; rounding it to `f32` first would get 8 of them wrong.
//!
//! Guards where TS would corrupt memory or hang (deviations, behaviour otherwise kept):
//! * `Load` stores a negative node count as is. TS's `AddNode` then writes the node before the
//!   array (outside the graph) and increments the count; here the node is dropped and the count
//!   incremented, so the count and the debug output match TS.
//! * `LinkNewNode` traces through `func_door`s forever if the engine keeps reporting a door
//!   (two overlapping doors, each trace ignoring only the last one). The port gives up after
//!   [`DOOR_TRACE_LIMIT`] traces and doesn't link the pair.
//! * `FindPath`'s debug printout builds `"path:%i,%i..."` in a 512-byte stack buffer that a long
//!   path overflows; the port's string has no limit (the chat message is clipped anyway).

use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::{DONT_IGNORE_MONSTERS, IGNORE_MONSTERS, MsgWriter, engine};
use goldsrc::sys::*;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;

/// Node limit (0x200; the checks at 0x10070e56 and 0x10071dcc). Also the path stack's size.
pub const MAX_NODES: usize = 512;
/// `CNode::flags` bit 0: learned on a ladder (`IsOnLadder` 0x10033670: movetype `MOVETYPE_FLY`).
pub const NODE_LADDER: i32 = 1;
/// "Infinity" for `FindPath`'s distances and the nearest-node searches (0x270f).
const INFINITY: i32 = 9999;
/// `FindPath`'s Dijkstra and walk iteration limit (0x3e8).
const MAX_ITERATIONS: i32 = 1000;
/// Floor probe depth of `IsInAir` / `IsGroundContinuous` (72.0 @ 0x1008f4e8).
const FLOOR_PROBE: f32 = 72.0;
/// Port guard: traces through doors per node pair before giving up (TS loops without limit).
pub const DOOR_TRACE_LIMIT: usize = 32;
/// `HUD_PRINTTALK`.
const HUD_PRINTTALK: u8 = 3;

/// `CNode` (0x14 bytes), also the on-disk node record.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Node {
    /// The player's `pev->origin` when learned (hull centre, not the eyes).
    pub origin: [f32; 3],
    /// [`NODE_LADDER`]; no other bit is ever set.
    pub flags: i32,
    /// Always 0.
    pub reserved: i32,
}

/// `CLink` (8 bytes), also the on-disk link record. `link[i][j]` and `link[j][i]` are equal.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Link {
    /// `__ftol(|origin_i - origin_j|)`: the 3D distance, truncated.
    pub dist: i32,
    /// 1 = passable.
    pub connected: u8,
    /// Never written (0 after `Reset`), but saved and loaded.
    pub pad: [u8; 3],
}

/// `NodeScratch` (16 bytes at +0x20400c): `FindPath`'s per-node state.
#[derive(Clone, Copy, Default, Debug)]
struct Scratch {
    /// Dijkstra distance from the start node.
    g: i32,
    /// Truncated distance to the goal node, computed lazily in the walk.
    h: i32,
    /// `g + h`; 0 = not computed yet.
    f: i32,
    closed: bool,
}

/// A trace's result as the graph needs it.
#[derive(Clone, Copy, Debug)]
pub struct NodeTrace {
    pub fraction: f32,
    pub end: [f32; 3],
    pub hit: *mut edict_t,
    /// `tr.pHit`'s classname is `func_door` or `func_door_rotating`.
    pub hit_door: bool,
}

/// What the graph needs from the engine.
pub trait NodeWorld {
    /// `UTIL_TraceLine(start, end, ignore_monsters ? ignore_monsters : dont_ignore_monsters, ignore)`.
    fn trace(&mut self, start: [f32; 3], end: [f32; 3], ignore_monsters: bool, ignore: *mut edict_t) -> NodeTrace;
    /// `TE_BEAMPOINTS` as `UTIL_DebugBeam` sends it (§11.6); `life` in 0.1 s.
    fn beam(&mut self, a: [f32; 3], b: [f32; 3], life: u8);
    /// `UTIL_ClientPrintAll(HUD_PRINTTALK, text)`.
    fn print_all(&mut self, text: &str);
}

/// MSVC `__ftol` (0x1007d310): truncation; out of range (and NaN) gives the integer indefinite.
fn ftol(x: f64) -> i32 {
    if x.is_nan() || x >= 2147483648.0 || x < -2147483648.0 { i32::MIN } else { x as i32 }
}

fn diff64(a: [f32; 3], b: [f32; 3]) -> [f64; 3] {
    [a[0] as f64 - b[0] as f64, a[1] as f64 - b[1] as f64, a[2] as f64 - b[2] as f64]
}

/// The x87 length sequence of `LinkNewNode` and `FindPath`: `(z² + x²) + y²`.
fn len_zxy(d: [f64; 3]) -> f64 {
    ((d[2] * d[2] + d[0] * d[0]) + d[1] * d[1]).sqrt()
}

/// The x87 length sequence of `IsTooClose` and `FindPathBetweenPoints`: `(z² + y²) + x²`.
fn len_zyx(d: [f64; 3]) -> f64 {
    ((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]).sqrt()
}

/// `UTIL_DebugBeam` (0x10070b80): skipped when either end is the zero vector.
fn debug_beam(w: &mut dyn NodeWorld, a: [f32; 3], b: [f32; 3], life: u8) {
    if a == [0.0; 3] || b == [0.0; 3] {
        return;
    }
    w.beam(a, b, life);
}

/// `UTIL_DebugNodeBeam` (0x10070cc0): a vertical beam from origin + 16 to origin - 16.
fn debug_node_beam(w: &mut dyn NodeWorld, o: [f32; 3], life: u8) {
    w.beam([o[0], o[1], o[2] + 16.0], [o[0], o[1], o[2] - 16.0], life);
}

/// `CNodeGraph` (0x20600c bytes @ 0x100e8330).
pub struct NodeGraph {
    /// `m_cNodes` (+0x0). Normally 0..=512; `Load` can store a negative value (see the module docs).
    count: i32,
    /// +0x4, [`MAX_NODES`] entries.
    nodes: Vec<Node>,
    /// +0x2804, row-major `MAX_NODES * MAX_NODES`.
    links: Vec<Link>,
    /// The path stack (+0x203808, count at +0x204008): at most [`MAX_NODES`] entries.
    stack: Vec<i32>,
    /// +0x20400c.
    scratch: Vec<Scratch>,
    /// Port: the last successful `FindPath`'s node indices, start first (empty after a failure).
    last_path: Vec<i32>,
    /// Port: the last `FindPath`'s walk iterations (the "(debug)%i passages" count).
    last_iterations: i32,
}

impl Default for NodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeGraph {
    /// The static constructor 0x10070dd0 -> ctor 0x10070df0: an empty graph.
    pub fn new() -> Self {
        NodeGraph {
            count: 0,
            nodes: vec![Node::default(); MAX_NODES],
            links: vec![Link::default(); MAX_NODES * MAX_NODES],
            stack: Vec::with_capacity(MAX_NODES),
            scratch: vec![Scratch::default(); MAX_NODES],
            last_path: Vec::new(),
            last_iterations: 0,
        }
    }

    /// `Reset` (0x10070e00): no nodes, nodes and links zeroed. (The scratch isn't cleared, as
    /// in TS; `FindPath` clears it before use.)
    pub fn reset(&mut self) {
        self.count = 0;
        self.nodes.fill(Node::default());
        self.links.fill(Link::default());
    }

    /// The node count (`m_cNodes`). Negative only after loading a file with a negative count.
    pub fn count(&self) -> i32 {
        self.count
    }

    /// The node count as a usize, 0 when negative.
    fn len(&self) -> usize {
        self.count.max(0) as usize
    }

    /// Node `i` (any slot below [`MAX_NODES`], used or not).
    pub fn node(&self, i: usize) -> &Node {
        &self.nodes[i]
    }

    /// The used nodes.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes[..self.len()]
    }

    /// `link[i][j]`.
    pub fn link(&self, i: usize, j: usize) -> &Link {
        &self.links[i * MAX_NODES + j]
    }

    /// The last successful [`find_path`](Self::find_path)'s node indices, start first; empty
    /// when it failed. (TS's path stack, read before the debug printout empties it.)
    pub fn last_path_indices(&self) -> &[i32] {
        &self.last_path
    }

    /// The last [`find_path`](Self::find_path)'s walk iterations ("(debug)%i passages").
    pub fn last_iterations(&self) -> i32 {
        self.last_iterations
    }

    /// `Load` (0x10071cb0) after the `fopen`: `Reset`, then the file if there is one (`None` =
    /// `fopen` failed). A count of 512 or more is refused (the graph stays empty, although
    /// `AddNode` and `Save` allow 512); a negative count is stored and nothing else is read. The
    /// `fread`s inside the loops aren't checked, so a truncated file leaves the rest zero (a
    /// partial record keeps the bytes that were there, as MSVC's `fread` copies them).
    pub fn load_bytes(&mut self, data: Option<&[u8]>) {
        self.reset();
        let Some(data) = data else { return };
        let Some(n) = data.get(..4) else { return };
        let n = i32::from_le_bytes(n.try_into().unwrap());
        if n >= MAX_NODES as i32 {
            return;
        }
        self.count = n;
        let mut pos = 4;
        let mut read = |buf: &mut [u8]| {
            let k = buf.len().min(data.len().saturating_sub(pos));
            buf[..k].copy_from_slice(&data[pos..pos + k]);
            pos += k;
        };
        let len = self.len();
        for node in &mut self.nodes[..len] {
            let mut b = node_bytes(node);
            read(&mut b);
            *node = node_from_bytes(&b);
        }
        for i in 0..len {
            for link in &mut self.links[i * MAX_NODES..i * MAX_NODES + len] {
                let mut b = link_bytes(link);
                read(&mut b);
                *link = link_from_bytes(&b);
            }
        }
    }

    /// `Save` (0x10071e60)'s file contents: `None` (nothing written) when there are no nodes.
    pub fn save_bytes(&self) -> Option<Vec<u8>> {
        if self.count <= 0 {
            return None;
        }
        let len = self.len();
        let mut out = Vec::with_capacity(4 + 20 * len + 8 * len * len);
        out.extend_from_slice(&self.count.to_le_bytes());
        for node in &self.nodes[..len] {
            out.extend_from_slice(&node_bytes(node));
        }
        for i in 0..len {
            for link in &self.links[i * MAX_NODES..i * MAX_NODES + len] {
                out.extend_from_slice(&link_bytes(link));
            }
        }
        Some(out)
    }

    /// `IsTooClose` (0x10070f50): a node closer than 8 (manual), 32 (the new node is a ladder
    /// node) or 128 units (strict, 3D).
    fn is_too_close(&self, n: &Node, manual: bool) -> bool {
        let r = if manual {
            8.0
        } else if n.flags & NODE_LADDER != 0 {
            32.0
        } else {
            128.0
        };
        self.nodes().iter().any(|o| len_zyx(diff64(o.origin, n.origin)) < r)
    }

    /// `AddNode` (0x10070e30): adds a node at `origin` unless it's too close to another one,
    /// in the air (not on a ladder, and no floor within 72 units below) or the graph is full,
    /// then links it. `manual` is the `addnode` command (8-unit spacing, debug output always);
    /// `debug` is `botlearn == 2`. True when the node was added.
    pub fn add_node(&mut self, w: &mut dyn NodeWorld, origin: [f32; 3], ladder: bool, manual: bool, debug: bool) -> bool {
        let n = Node { origin, flags: if ladder { NODE_LADDER } else { 0 }, reserved: 0 };
        if self.is_too_close(&n, manual) || is_in_air(w, &n) || self.count >= MAX_NODES as i32 {
            return false;
        }
        if self.count >= 0 {
            self.nodes[self.count as usize] = n;
        }
        // A negative count: TS writes the node before the array (see the module docs).
        self.count += 1;
        self.link_new_node(w, self.count - 1, manual, debug);
        if debug || manual {
            debug_node_beam(w, origin, 10);
            w.print_all(&format!("(debug){} nodes\n", self.count));
        }
        true
    }

    /// `LinkNewNode` (0x10071410): links node `i` to every other node it sees (tracing through
    /// `func_door`s) and can reach ([`is_reachable`](Self::is_reachable)), both directions.
    fn link_new_node(&mut self, w: &mut dyn NodeWorld, i: i32, manual: bool, debug: bool) {
        if self.count < 1 {
            return;
        }
        let i = i as usize;
        for j in 0..self.len() {
            if j == i {
                continue;
            }
            let (from, to) = (self.nodes[i].origin, self.nodes[j].origin);
            let mut start = from;
            let mut ignore: *mut edict_t = std::ptr::null_mut();
            let mut tr = w.trace(start, to, true, ignore);
            let mut doors = 0;
            while tr.hit_door && doors < DOOR_TRACE_LIMIT {
                start = tr.end;
                ignore = tr.hit;
                tr = w.trace(start, to, true, ignore);
                doors += 1;
            }
            if tr.hit_door || tr.fraction < 1.0 || !self.is_reachable(w, i, j) {
                continue;
            }
            let link = Link { dist: ftol(len_zxy(diff64(from, to))), connected: 1, pad: self.links[i * MAX_NODES + j].pad };
            self.links[i * MAX_NODES + j] = link;
            self.links[j * MAX_NODES + i] = link;
            if debug || manual {
                debug_beam(w, from, to, 10);
            }
        }
    }

    /// `IsReachable` (0x100711e0): two ground nodes need [`is_ground_continuous`]; with a ladder
    /// node a 2D distance under 64, or under 160 with less than 32 units of height, is enough.
    fn is_reachable(&self, w: &mut dyn NodeWorld, i: usize, j: usize) -> bool {
        let (a, b) = (&self.nodes[i], &self.nodes[j]);
        if (a.flags | b.flags) & NODE_LADDER != 0 {
            // The differences are stored as floats; the squares and sqrt stay on the x87 stack.
            let dx = (a.origin[0] - b.origin[0]) as f64;
            let dy = (a.origin[1] - b.origin[1]) as f64;
            let d2 = (dy * dy + dx * dx).sqrt();
            if d2 < 64.0 {
                return true;
            }
            if d2 < 160.0 && (a.origin[2] as f64 - b.origin[2] as f64).abs() < 32.0 {
                return true;
            }
        }
        is_ground_continuous(w, a.origin, b.origin)
    }

    /// `FindPath` (0x100717f0): a Dijkstra from `start`, then a greedy walk from `goal` back to
    /// `start`. True when the path has at least 2 nodes; `out` (when given) then gets the node
    /// origins, start first, and is left alone on failure. `debug` (`botlearn == 2`, as for
    /// `testpath`) prints "(debug)arrived", "(debug)no node found", "(debug)%i passages" and the
    /// "path:..." list with 10 s beams.
    ///
    /// Kept quirks: ties go to the lowest index; both loops stop after 1000 iterations (the walk
    /// then "succeeds" without reaching `start`); closed nodes are still relaxed; a walk
    /// candidate needs both `g < bestG` and `f < bestF`; `h` is the distance to `goal`; a dead
    /// end pops the current node itself (so the path can skip a node and contain an unlinked
    /// hop); pushes beyond 512 entries are dropped.
    pub fn find_path(&mut self, w: &mut dyn NodeWorld, start: i32, goal: i32, out: Option<&mut Vec<[f32; 3]>>, debug: bool) -> bool {
        self.last_path.clear();
        self.last_iterations = 0;
        if start == goal || goal >= self.count || goal < 0 || start >= self.count || start < 0 {
            return false;
        }
        let (start, goal, n) = (start as usize, goal as usize, self.len());
        // ClearScratch (0x10071650).
        self.stack.clear();
        self.scratch.fill(Scratch::default());
        for s in &mut self.scratch[..n] {
            s.g = INFINITY;
        }
        self.scratch[start].g = 0;

        // Phase 1: Dijkstra; the closed flag marks finished nodes.
        for _ in 0..MAX_ITERATIONS {
            let (mut u, mut best) = (0, INFINITY);
            for (i, s) in self.scratch[..n].iter().enumerate() {
                if !s.closed && s.g < best {
                    (u, best) = (i, s.g);
                }
            }
            // With nothing left, node 0 is closed.
            self.scratch[u].closed = true;
            if u == goal || best == INFINITY {
                break;
            }
            let gu = self.scratch[u].g;
            for (s, l) in self.scratch[..n].iter_mut().zip(&self.links[u * MAX_NODES..u * MAX_NODES + n]) {
                if l.connected != 0 && s.g > gu.wrapping_add(l.dist) {
                    s.g = gu.wrapping_add(l.dist);
                }
            }
        }

        // Phase 2: walk from the goal to the start.
        for s in &mut self.scratch[..n] {
            s.closed = false;
        }
        let goal_origin = self.nodes[goal].origin;
        let mut cur = goal as i32;
        self.scratch[goal].closed = true;
        self.push(goal as i32);
        let mut it = 0;
        let mut arrived = false;
        loop {
            let (mut best_g, mut best_f, mut bi) = (INFINITY, INFINITY, -1);
            let c = cur as usize;
            for j in 0..n {
                if self.scratch[j].closed || j == c || self.links[c * MAX_NODES + j].connected == 0 {
                    continue;
                }
                if j == start {
                    self.push(j as i32);
                    arrived = true;
                    break;
                }
                let s = &mut self.scratch[j];
                if s.f == 0 {
                    s.h = ftol(len_zxy(diff64(goal_origin, self.nodes[j].origin)));
                    s.f = s.h.wrapping_add(s.g);
                }
                if s.g < best_g && s.f < best_f {
                    (best_g, best_f, bi) = (s.g, s.f, j as i32);
                }
            }
            if arrived {
                if debug {
                    w.print_all("(debug)arrived\n");
                }
                break;
            }
            if bi == -1 {
                if self.stack.len() > 1 {
                    self.scratch[c].closed = true;
                    // PopPath (0x10071620) returns the top of the stack, which is `cur` itself.
                    cur = self.pop();
                    self.scratch[cur as usize].closed = false;
                } else {
                    if debug {
                        w.print_all("(debug)no node found\n");
                    }
                    cur = -1;
                }
            } else {
                self.scratch[bi as usize].closed = true;
                self.push(bi);
                cur = bi;
            }
            it += 1;
            if it >= MAX_ITERATIONS || cur == -1 {
                break;
            }
        }
        self.last_iterations = it;
        if !arrived && debug {
            w.print_all(&format!("(debug){it} passages\n"));
        }

        if self.stack.len() <= 1 {
            return false;
        }
        self.last_path.extend(self.stack.iter().rev());
        if let Some(out) = out {
            out.clear();
            out.extend(self.last_path.iter().map(|&i| self.nodes[i as usize].origin));
        }
        if debug {
            // Pops the whole stack, as TS does.
            let mut a = self.pop();
            let mut text = format!("path:{a}");
            loop {
                let b = self.pop();
                text.push_str(&format!(",{b}"));
                if a != -1 && b != -1 {
                    debug_beam(w, self.nodes[a as usize].origin, self.nodes[b as usize].origin, 100);
                }
                a = b;
                if self.stack.is_empty() {
                    break;
                }
            }
            text.push_str(" \n");
            w.print_all(&text);
        }
        true
    }

    /// `PushPath` (0x100715f0): ignored when the stack is full.
    fn push(&mut self, i: i32) {
        if self.stack.len() < MAX_NODES {
            self.stack.push(i);
        }
    }

    /// `PopPath` (0x10071620): the top, or -1 when empty.
    fn pop(&mut self) -> i32 {
        self.stack.pop().unwrap_or(-1)
    }

    /// `FindPathBetweenPoints` (0x100716a0): [`find_path`](Self::find_path) from the node
    /// nearest to `from` to the node nearest to `to`, each the closest node with a clear trace
    /// (no distance limit, but 9999 units or more never count; strict `<`, so the lowest index
    /// wins ties). No visible node fails. The path ends at a node, never at `to` itself.
    pub fn find_path_between_points(
        &mut self,
        w: &mut dyn NodeWorld,
        from: [f32; 3],
        to: [f32; 3],
        out: &mut Vec<[f32; 3]>,
        ignore: *mut edict_t,
        debug: bool,
    ) -> bool {
        let (mut best_a, mut best_b, mut ia, mut ib) = (INFINITY, INFINITY, -1, -1);
        for i in 0..self.len() {
            let o = self.nodes[i].origin;
            if w.trace(from, o, true, ignore).fraction >= 1.0 {
                let d = ftol(len_zyx(diff64(from, o)));
                if d < best_a {
                    (best_a, ia) = (d, i as i32);
                }
            }
            if w.trace(to, o, true, ignore).fraction >= 1.0 {
                let d = ftol(len_zyx(diff64(to, o)));
                if d < best_b {
                    (best_b, ib) = (d, i as i32);
                }
            }
        }
        self.find_path(w, ia, ib, Some(out), debug)
    }
}

/// `IsInAir` (0x10070ed0): a non-ladder node with no floor within 72 units below.
fn is_in_air(w: &mut dyn NodeWorld, n: &Node) -> bool {
    if n.flags & NODE_LADDER != 0 {
        return false;
    }
    let o = n.origin;
    w.trace(o, [o[0], o[1], o[2] - FLOOR_PROBE], true, std::ptr::null_mut()).fraction >= 1.0
}

/// `IsGroundContinuous` (0x10070fe0, `a` = the new node): floor within 72 units below the line
/// every 16 units (`__ftol(|b - a| / 16)` probes; none when that's 0).
fn is_ground_continuous(w: &mut dyn NodeWorld, a: [f32; 3], b: [f32; 3]) -> bool {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let (x, y, z) = (d[0] as f64, d[1] as f64, d[2] as f64);
    let steps = ftol(((x * x + y * y) + z * z).sqrt() * 0.0625);
    // The divisions round to float; with 0 steps they give inf/NaN, which the loop never uses.
    let s = d.map(|c| (c as f64 / steps as f64) as f32);
    for k in 1..=steps {
        let fk = k as f32;
        let p = [s[0] * fk + a[0], s[1] * fk + a[1], s[2] * fk + a[2]];
        if w.trace(p, [p[0], p[1], p[2] - FLOOR_PROBE], true, std::ptr::null_mut()).fraction >= 1.0 {
            return false;
        }
    }
    true
}

fn node_bytes(n: &Node) -> [u8; 20] {
    let mut b = [0; 20];
    for (k, c) in n.origin.iter().enumerate() {
        b[k * 4..k * 4 + 4].copy_from_slice(&c.to_le_bytes());
    }
    b[12..16].copy_from_slice(&n.flags.to_le_bytes());
    b[16..20].copy_from_slice(&n.reserved.to_le_bytes());
    b
}

fn node_from_bytes(b: &[u8; 20]) -> Node {
    let f = |k: usize| f32::from_le_bytes(b[k..k + 4].try_into().unwrap());
    let i = |k: usize| i32::from_le_bytes(b[k..k + 4].try_into().unwrap());
    Node { origin: [f(0), f(4), f(8)], flags: i(12), reserved: i(16) }
}

fn link_bytes(l: &Link) -> [u8; 8] {
    let mut b = [0; 8];
    b[..4].copy_from_slice(&l.dist.to_le_bytes());
    b[4] = l.connected;
    b[5..].copy_from_slice(&l.pad);
    b
}

fn link_from_bytes(b: &[u8; 8]) -> Link {
    Link { dist: i32::from_le_bytes(b[..4].try_into().unwrap()), connected: b[4], pad: [b[5], b[6], b[7]] }
}

// ---- engine side ------------------------------------------------------------------------------

/// `g_sModelIndexLightning` (DAT_102ee33c): mp.dll's `W_Precache` (0x100799c0) precaches
/// `sprites/lgtning.spr` for it. 0 = not precached: the beams are skipped (port guard).
static LIGHTNING: Global<Cell<i32>> = Global(Cell::new(0));

/// Sets `g_sModelIndexLightning` (the precache of `sprites/lgtning.spr`).
pub fn set_lightning_sprite(index: i32) {
    LIGHTNING.0.set(index);
}

/// The real engine as a [`NodeWorld`].
pub struct EngineWorld;

impl NodeWorld for EngineWorld {
    /// `UTIL_TraceLine` (0x10076fb0); the door test is `LinkNewNode`'s `strcmp`s on
    /// `STRING(tr.pHit->v.classname)`.
    fn trace(&mut self, start: [f32; 3], end: [f32; 3], ignore_monsters: bool, ignore: *mut edict_t) -> NodeTrace {
        let eng = engine();
        let tr = eng.trace_line(start, end, if ignore_monsters { IGNORE_MONSTERS } else { DONT_IGNORE_MONSTERS }, ignore);
        let hit_door = !tr.pHit.is_null() && {
            let class = eng.string(tr.pHit.v().classname);
            class == c"func_door" || class == c"func_door_rotating"
        };
        NodeTrace { fraction: tr.flFraction, end: tr.vecEndPos, hit: tr.pHit, hit_door }
    }

    /// The `TE_BEAMPOINTS` message of `UTIL_DebugBeam` (0x10070b80) / `UTIL_DebugNodeBeam`
    /// (0x10070cc0): `MSG_ALL`, lightning sprite, frame 1, framerate 10, width 30, noise 0,
    /// colour (0, 255, 50), brightness 200, scroll 10.
    fn beam(&mut self, a: [f32; 3], b: [f32; 3], life: u8) {
        let sprite = LIGHTNING.0.get();
        if sprite == 0 {
            return;
        }
        let mut m = MsgWriter::begin(MSG_ALL as i32, goldsrc::ext::entities::SVC_TEMPENTITY, None, std::ptr::null_mut())
            .byte(goldsrc::ext::entities::te::BEAMPOINTS);
        for c in a.into_iter().chain(b) {
            m = m.coord(c);
        }
        m.short(sprite).byte(1).byte(10).byte(life as i32).byte(30).byte(0).byte(0).byte(255).byte(50).byte(200).byte(10).end();
    }

    /// `UTIL_ClientPrintAll` (0x10076cf0).
    fn print_all(&mut self, text: &str) {
        crate::rules::msg::text_all(HUD_PRINTTALK, text);
    }
}

static GRAPH: Global<RefCell<Option<NodeGraph>>> = Global(RefCell::new(None));

/// Runs `f` on the server's graph (`g_NodeGraph`), allocated on first use (it's ~2 MB).
pub fn with<R>(f: impl FnOnce(&mut NodeGraph) -> R) -> R {
    let mut g = GRAPH.0.borrow_mut();
    f(g.get_or_insert_with(NodeGraph::new))
}

/// `botlearn` (registered by the rules cvars).
fn botlearn() -> f32 {
    crate::rules::cvars::f(c"botlearn")
}

/// `<gamedir>/maps/<mapname>.ndm`.
fn ndm_path() -> PathBuf {
    let eng = engine();
    let map = eng.string(eng.globals().mapname).to_string_lossy().into_owned();
    PathBuf::from(eng.game_dir()).join("maps").join(format!("{map}.ndm"))
}

/// `Load` (0x10071cb0): `Reset`, then `<gamedir>/maps/<map>.ndm` if it exists, read relative to
/// the engine's working directory as TS's `fopen` does. `CTSGameRules::Init` (0x1004d910) calls
/// it at every map start (its `Reset` first is folded in here); also the `loadnodes` command.
pub fn map_init() {
    let data = std::fs::read(ndm_path()).ok();
    with(|g| g.load_bytes(data.as_deref()));
}

/// `Save` (0x10071e60): writes the graph to `<gamedir>/maps/<map>.ndm`; nothing when there are
/// no nodes, silent on failure. TS writes the hard-coded `TS\maps\<map>.ndm`, which on Windows'
/// case-insensitive file system is the game dir of the shipped mod (`ts`), i.e. the file `Load`
/// reads; the port uses the game dir, the portable equivalent.
pub fn save() {
    if let Some(bytes) = with(|g| g.save_bytes()) {
        let _ = std::fs::write(ndm_path(), bytes);
    }
}

/// The node learning block of `CBasePlayer::PreThink` (0x1005fed2..0x1005ff58), for every
/// player (bots included) that gets that far: with `botlearn > 0`, every 0.1 s
/// (`m_flNextNodeLearn`, pl+0x774) the player's origin (and ladder state) is offered to
/// `AddNode`, with debug output when `botlearn == 2`.
pub fn learn(next_learn: &mut f32, now: f32, origin: [f32; 3], ladder: bool) {
    let botlearn = botlearn();
    if botlearn > 0.0 && *next_learn < now {
        *next_learn = (now as f64 + 0.1) as f32;
        with(|g| g.add_node(&mut EngineWorld, origin, ladder, false, botlearn == 2.0));
    }
}

/// C `atoi` (0x1007d419): leading white space, a sign, digits; wraps on overflow.
fn atoi(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = b.iter().take_while(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)).count();
    let neg = b.get(i) == Some(&b'-');
    if matches!(b.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let mut v: i32 = 0;
    for c in b[i..].iter().take_while(|c| c.is_ascii_digit()) {
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i32);
    }
    if neg { v.wrapping_neg() } else { v }
}

/// The node commands of `pfnClientCommand` (0x1000f8a0); true when `cmd` was one. `args` is the
/// whole argv (`args[0]` = `cmd`). Any client may run them (TS has no access check):
/// * `testpath a b` (needs 2 arguments): `FindPath(atoi(a), atoi(b))`, output only with
///   `botlearn 2`;
/// * `resetnodes`, `loadnodes`, `savenodes`;
/// * `addnode`: when the player is alive and not observing (`alive_and_playing`), a manual node
///   at `origin` (8-unit spacing, debug output).
pub fn client_command(cmd: &str, args: &[String], alive_and_playing: bool, origin: [f32; 3], ladder: bool) -> bool {
    match cmd {
        "testpath" => {
            if args.len() > 2 {
                let (a, b) = (atoi(&args[1]), atoi(&args[2]));
                let debug = botlearn() == 2.0;
                with(|g| g.find_path(&mut EngineWorld, a, b, None, debug));
            }
        }
        "resetnodes" => with(|g| g.reset()),
        "loadnodes" => map_init(),
        "savenodes" => save(),
        "addnode" => {
            if alive_and_playing {
                let debug = botlearn() == 2.0;
                with(|g| g.add_node(&mut EngineWorld, origin, ladder, true, debug));
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ---- helpers ----------------------------------------------------------------------------

    /// A world with a floor at z = 0 (except in `holes`), vertical walls and doors at `x = c`,
    /// recording every call.
    #[derive(Default)]
    struct Fake {
        /// `[x0, x1, y0, y1]`: no floor.
        holes: Vec<[f32; 4]>,
        /// `(x, y0, y1)`: a wall in the plane x between y0 and y1.
        walls: Vec<(f32, f32, f32)>,
        /// Door planes `x = c` (all y); door k's edict is [`door`]`(k)`.
        doors: Vec<f32>,
        /// Every horizontal trace is blocked at its start.
        opaque: bool,
        traces: Vec<([f32; 3], [f32; 3], *mut edict_t)>,
        beams: Vec<([f32; 3], [f32; 3], u8)>,
        prints: Vec<String>,
    }

    fn door(k: usize) -> *mut edict_t {
        std::ptr::without_provenance_mut(0x1000 + k * 0x100)
    }

    fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    }

    impl NodeWorld for Fake {
        fn trace(&mut self, start: [f32; 3], end: [f32; 3], ignore_monsters: bool, ignore: *mut edict_t) -> NodeTrace {
            assert!(ignore_monsters, "the graph's traces all ignore monsters");
            self.traces.push((start, end, ignore));
            let (mut best, mut hit, mut hit_door) = (1.0f32, std::ptr::null_mut(), false);
            if self.opaque && (start[0], start[1]) != (end[0], end[1]) {
                return NodeTrace { fraction: 0.0, end: start, hit, hit_door };
            }
            if start[2] > 0.0 && end[2] <= 0.0 {
                let t = start[2] / (start[2] - end[2]);
                let p = lerp(start, end, t);
                if !self.holes.iter().any(|h| (h[0]..=h[1]).contains(&p[0]) && (h[2]..=h[3]).contains(&p[1])) {
                    best = t;
                }
            }
            for &(c, y0, y1) in &self.walls {
                if (start[0] - c) * (end[0] - c) < 0.0 {
                    let t = (c - start[0]) / (end[0] - start[0]);
                    if (y0..=y1).contains(&lerp(start, end, t)[1]) && t < best {
                        (best, hit_door) = (t, false);
                    }
                }
            }
            for (k, &c) in self.doors.iter().enumerate() {
                // A trace starting in the door plane hits it too (a door the trace starts in).
                if door(k) != ignore && start[0] != end[0] && (start[0] - c) * (end[0] - c) <= 0.0 {
                    let t = (c - start[0]) / (end[0] - start[0]);
                    if t < best {
                        (best, hit, hit_door) = (t, door(k), true);
                    }
                }
            }
            NodeTrace { fraction: best, end: lerp(start, end, best), hit, hit_door }
        }
        fn beam(&mut self, a: [f32; 3], b: [f32; 3], life: u8) {
            self.beams.push((a, b, life));
        }
        fn print_all(&mut self, text: &str) {
            self.prints.push(text.to_owned());
        }
    }

    /// A graph file from node origins (`ladders` flagged) and undirected links.
    fn graph_file(nodes: &[[f32; 3]], ladders: &[usize], links: &[(usize, usize)]) -> Vec<u8> {
        let n = nodes.len();
        let mut out = (n as i32).to_le_bytes().to_vec();
        for (i, o) in nodes.iter().enumerate() {
            out.extend_from_slice(&node_bytes(&Node { origin: *o, flags: ladders.contains(&i) as i32, reserved: 0 }));
        }
        for i in 0..n {
            for j in 0..n {
                let linked = links.contains(&(i, j)) || links.contains(&(j, i));
                let dist = if linked { ftol(len_zxy(diff64(nodes[i], nodes[j]))) } else { 0 };
                out.extend_from_slice(&link_bytes(&Link { dist, connected: linked as u8, pad: [0; 3] }));
            }
        }
        out
    }

    fn graph(nodes: &[[f32; 3]], links: &[(usize, usize)]) -> NodeGraph {
        let mut g = NodeGraph::new();
        g.load_bytes(Some(&graph_file(nodes, &[], links)));
        g
    }

    fn content(name: &str) -> Option<std::path::PathBuf> {
        Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().skip(1).map(|d| d.join(name)).find(|p| p.is_dir())
    }

    /// A shipped `.ndm`, from `ts_original/maps` or `ts_custom/maps` next to the checkout.
    fn shipped(map: &str) -> Option<Vec<u8>> {
        ["ts_original", "ts_custom"]
            .iter()
            .filter_map(|d| content(d))
            .find_map(|d| std::fs::read(d.join("maps").join(format!("{map}.ndm"))).ok())
    }

    // ---- the shipped files ------------------------------------------------------------------

    /// bots.md §11.4: (map, nodes, undirected links, ladder nodes).
    const SHIPPED: &[(&str, i32, usize, usize)] = &[
        ("ts_awaken", 227, 1897, 0),
        ("ts_bikini", 255, 1943, 0),
        ("ts_casa", 345, 1936, 1),
        ("ts_central", 484, 5077, 4),
        ("ts_chateau", 194, 835, 0),
        ("ts_dietrich", 352, 2924, 0),
        ("ts_dojo", 120, 1835, 0),
        ("ts_faculty", 481, 2628, 0),
        ("ts_fall", 211, 1181, 58),
        ("ts_gpc", 146, 1213, 0),
        ("ts_greenhouse", 256, 2223, 3),
        ("ts_hammertime", 132, 591, 0),
        ("ts_henningsen", 202, 1458, 0),
        ("ts_heresy", 238, 910, 38),
        ("ts_lobbyduel", 154, 1669, 0),
        ("ts_metro", 179, 1098, 0),
        ("ts_panzibar", 340, 2457, 0),
        ("ts_parking", 442, 3537, 23),
        ("ts_rundown", 145, 1033, 0),
        ("ts_urbannights", 196, 583, 6),
        ("tm_stormshield", 248, 1407, 0),
        ("ts_cave_of_fear", 471, 23723, 0),
        ("ts_japa", 97, 666, 0),
    ];

    #[test]
    fn shipped_files_load_and_save_byte_for_byte() {
        let dirs: Vec<_> = ["ts_original", "ts_custom"].iter().filter_map(|d| content(d)).collect();
        if dirs.is_empty() {
            eprintln!("note: no ts_original / ts_custom next to the checkout; shipped .ndm files not checked");
            return;
        }
        let mut checked = 0;
        for dir in dirs {
            let mut files: Vec<_> = std::fs::read_dir(dir.join("maps"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "ndm"))
                .collect();
            files.sort();
            for file in files {
                let map = file.file_stem().unwrap().to_string_lossy().into_owned();
                let data = std::fs::read(&file).unwrap();
                let mut g = NodeGraph::new();
                g.load_bytes(Some(&data));
                let &(_, n, links, ladders) = SHIPPED.iter().find(|s| s.0 == map).unwrap_or_else(|| panic!("{map} not in bots.md's table"));
                assert_eq!(g.count(), n, "{map}");
                let n = n as usize;
                let mut directed = 0;
                for i in 0..n {
                    for j in 0..n {
                        let l = *g.link(i, j);
                        assert_eq!(l, *g.link(j, i), "{map} {i} {j}");
                        if l.connected != 0 {
                            directed += 1;
                            // The stored distance is the unrounded x87 length, truncated.
                            assert_eq!(l.dist, ftol(len_zxy(diff64(g.node(i).origin, g.node(j).origin))), "{map} {i} {j}");
                        }
                    }
                }
                assert_eq!(directed / 2, links, "{map}");
                assert_eq!(g.nodes().iter().filter(|n| n.flags & NODE_LADDER != 0).count(), ladders, "{map}");
                assert_eq!(g.save_bytes().as_deref(), Some(&data[..]), "{map}");
                checked += 1;
            }
        }
        eprintln!("checked {checked} shipped .ndm files");
    }

    // ---- Load / Save quirks -----------------------------------------------------------------

    #[test]
    fn load_refuses_512_nodes_and_accepts_511() {
        let nodes: Vec<[f32; 3]> = (0..512).map(|i| [i as f32 * 200.0, 0.0, 36.0]).collect();
        let mut g = NodeGraph::new();
        g.load_bytes(Some(&graph_file(&nodes, &[], &[(0, 1)])));
        assert_eq!(g.count(), 0);
        assert_eq!(*g.node(0), Node::default());
        assert_eq!(g.save_bytes(), None);

        let file = graph_file(&nodes[..511], &[3], &[(0, 1), (509, 510)]);
        g.load_bytes(Some(&file));
        assert_eq!(g.count(), 511);
        assert_eq!(g.node(510).origin, [102000.0, 0.0, 36.0]);
        assert_eq!(g.node(3).flags, NODE_LADDER);
        assert_eq!(*g.link(510, 509), Link { dist: 200, connected: 1, pad: [0; 3] });
        assert_eq!(g.save_bytes().unwrap(), file);
    }

    #[test]
    fn load_negative_count() {
        let mut g = graph(&[[1.0, 2.0, 3.0]], &[]);
        let mut file = (-3i32).to_le_bytes().to_vec();
        file.extend_from_slice(&[0x55; 100]);
        g.load_bytes(Some(&file));
        assert_eq!(g.count(), -3);
        assert!(g.nodes().is_empty());
        assert_eq!(*g.node(0), Node::default(), "Load resets first and reads nothing else");
        assert_eq!(g.save_bytes(), None);
        let mut w = Fake::default();
        assert!(!g.find_path(&mut w, 0, 1, None, true));
        assert!(w.prints.is_empty());
        // TS's AddNode writes these three nodes outside the array; the count still goes up.
        for (k, x) in [0.0f32, 1.0, 2.0].into_iter().enumerate() {
            assert!(g.add_node(&mut w, [x, 0.0, 36.0], false, true, false));
            assert_eq!(w.prints.last().unwrap(), &format!("(debug){} nodes\n", k as i32 - 2));
        }
        assert_eq!(g.count(), 0);
        assert_eq!(*g.node(0), Node::default());
        assert!(g.add_node(&mut w, [3.0, 0.0, 36.0], false, false, false));
        assert_eq!(g.count(), 1);
        assert_eq!(g.node(0).origin, [3.0, 0.0, 36.0]);
    }

    #[test]
    fn load_truncated_and_empty_files() {
        let nodes = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
        let file = graph_file(&nodes, &[], &[(0, 1), (1, 2)]);
        let mut g = NodeGraph::new();
        // Two and a half nodes: the half record keeps its x and y and half of z's bytes.
        g.load_bytes(Some(&file[..4 + 20 * 2 + 10]));
        assert_eq!(g.count(), 3);
        assert_eq!(g.node(1).origin, [4.0, 5.0, 6.0]);
        let mut half = node_bytes(&Node { origin: nodes[2], ..Default::default() });
        half[10..].fill(0);
        assert_eq!(*g.node(2), node_from_bytes(&half));
        assert_eq!(g.node(2).origin[..2], [7.0, 8.0]);
        assert!((0..3).all(|i| (0..3).all(|j| *g.link(i, j) == Link::default())));
        // Cut in the links: the first row is there, the rest is zero.
        g.load_bytes(Some(&file[..4 + 60 + 8 * 3]));
        assert_eq!(g.link(0, 1).connected, 1);
        assert_eq!(*g.link(1, 0), Link::default());
        // Empty, short and missing files leave an empty graph.
        for data in [Some(&file[..0]), Some(&file[..3]), None] {
            g.load_bytes(Some(&file));
            g.load_bytes(data);
            assert_eq!(g.count(), 0);
            assert_eq!(*g.node(0), Node::default());
            assert_eq!(*g.link(0, 1), Link::default());
        }
    }

    // ---- FindPath against the replica ---------------------------------------------------------

    fn fnv(mut h: u64, data: &[u8]) -> u64 {
        for &b in data {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    const FNV_BASIS: u64 = 0xcbf29ce484222325;

    fn digest(h: u64, ok: bool, path: &[i32]) -> u64 {
        let mut b = vec![ok as u8];
        b.extend_from_slice(&(path.len() as u16).to_le_bytes());
        for &i in path {
            b.extend_from_slice(&(i as u16).to_le_bytes());
        }
        fnv(h, &b)
    }

    fn lcg(x: &mut u64, n: u64) -> i32 {
        *x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*x >> 33) % n) as i32
    }

    /// Compares `FindPath` with the reference `testdata/gen.py` made from `scripts/ndm.py`.
    fn check_reference(reference: &str) {
        let mut lines = reference.lines().map(|l| l.split(' ').collect::<Vec<_>>());
        let head = lines.next().unwrap();
        let (map, n) = (head[1], head[2].parse::<i32>().unwrap());
        let Some(data) = shipped(map) else {
            eprintln!("note: {map}.ndm not found next to the checkout; FindPath reference not checked");
            return;
        };
        let mut g = NodeGraph::new();
        g.load_bytes(Some(&data));
        assert_eq!(g.count(), n);
        let mut w = Fake::default();
        let (mut pairs, mut paths) = (0, 0);
        let mut sampled: Option<(u64, usize)> = None;
        for l in lines {
            match l[0] {
                "row" => {
                    let a = l[1].parse().unwrap();
                    let mut h = FNV_BASIS;
                    for b in 0..n {
                        let ok = g.find_path(&mut w, a, b, None, false);
                        h = digest(h, ok, g.last_path_indices());
                    }
                    assert_eq!(format!("{h:016x}"), l[2], "{map} row {a}");
                    pairs += n;
                }
                "pairs" => {
                    let (seed, chunk) = (l[1].parse().unwrap(), l[3].parse().unwrap());
                    sampled = Some((seed, chunk));
                }
                "chunk" => {
                    let (x, chunk) = sampled.as_mut().unwrap();
                    let mut h = FNV_BASIS;
                    for _ in 0..*chunk {
                        let (a, b) = (lcg(x, n as u64), lcg(x, n as u64));
                        let ok = g.find_path(&mut w, a, b, None, false);
                        h = digest(h, ok, g.last_path_indices());
                    }
                    assert_eq!(format!("{h:016x}"), l[2], "{map} chunk {}", l[1]);
                    pairs += *chunk as i32;
                }
                "path" => {
                    let (a, b) = (l[1].parse().unwrap(), l[2].parse().unwrap());
                    let mut out = vec![[1.0; 3]];
                    let ok = g.find_path(&mut w, a, b, Some(&mut out), false);
                    assert_eq!(ok, l[3] == "1", "{map} {a}->{b}");
                    assert_eq!(g.last_iterations().to_string(), l[4], "{map} {a}->{b}");
                    let want: Vec<i32> = if l[5] == "-" { vec![] } else { l[5].split(',').map(|i| i.parse().unwrap()).collect() };
                    assert_eq!(g.last_path_indices(), &want[..], "{map} {a}->{b}");
                    if ok {
                        assert_eq!(out, want.iter().map(|&i| g.node(i as usize).origin).collect::<Vec<_>>());
                    } else {
                        assert_eq!(out, vec![[1.0; 3]], "a failed FindPath leaves the output alone");
                    }
                    paths += 1;
                }
                _ => panic!("bad reference line {l:?}"),
            }
        }
        assert!(w.traces.is_empty() && w.beams.is_empty() && w.prints.is_empty());
        eprintln!("{map}: {pairs} pairs by digest, {paths} full paths match");
    }

    #[test]
    fn find_path_matches_reference_ts_dojo() {
        check_reference(include_str!("testdata/ts_dojo.txt"));
    }

    #[test]
    fn find_path_matches_reference_ts_japa() {
        check_reference(include_str!("testdata/ts_japa.txt"));
    }

    #[test]
    fn find_path_matches_reference_ts_fall() {
        check_reference(include_str!("testdata/ts_fall.txt"));
    }

    #[test]
    fn find_path_matches_reference_ts_central() {
        check_reference(include_str!("testdata/ts_central.txt"));
    }

    #[test]
    fn find_path_matches_reference_ts_cave_of_fear() {
        check_reference(include_str!("testdata/ts_cave_of_fear.txt"));
    }

    #[test]
    fn find_path_matches_reference_ts_urbannights() {
        check_reference(include_str!("testdata/ts_urbannights.txt"));
    }

    // ---- FindPath on small graphs -------------------------------------------------------------

    #[test]
    fn find_path_rejects_bad_indices() {
        let mut g = graph(&[[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], &[(0, 1)]);
        let mut w = Fake::default();
        for (a, b) in [(0, 0), (-1, 1), (0, 2), (2, 0), (0, -1)] {
            assert!(!g.find_path(&mut w, a, b, None, true), "{a} {b}");
        }
        assert!(w.prints.is_empty());
        assert!(g.find_path(&mut w, 1, 0, None, false));
        assert_eq!(g.last_path_indices(), [1, 0]);
    }

    #[test]
    fn find_path_debug_output() {
        let nodes = [[10.0, 0.0, 36.0], [128.0, 0.0, 36.0], [128.0, 128.0, 36.0], [900.0, 900.0, 36.0]];
        let mut g = graph(&nodes, &[(0, 1), (1, 2)]);
        let mut w = Fake::default();
        let mut out = Vec::new();
        assert!(g.find_path(&mut w, 0, 2, Some(&mut out), true));
        assert_eq!(out, nodes[..3]);
        assert_eq!(g.last_path_indices(), [0, 1, 2]);
        assert_eq!(w.prints, ["(debug)arrived\n", "path:0,1,2 \n"]);
        assert_eq!(w.beams, [(nodes[0], nodes[1], 100), (nodes[1], nodes[2], 100)]);

        // Unreachable goal: the walk finds nothing at the goal.
        let (mut w, mut out) = (Fake::default(), vec![[5.0; 3]]);
        assert!(!g.find_path(&mut w, 0, 3, Some(&mut out), true));
        assert_eq!(w.prints, ["(debug)no node found\n", "(debug)1 passages\n"]);
        assert_eq!(out, [[5.0; 3]]);
        assert!(g.last_path_indices().is_empty());
        assert_eq!(g.last_iterations(), 1);

        // A node at the origin gets no beam (UTIL_DebugBeam skips g_vecZero).
        let mut g = graph(&[[0.0; 3], [100.0, 0.0, 0.0]], &[(0, 1)]);
        let mut w = Fake::default();
        assert!(g.find_path(&mut w, 1, 0, None, true));
        assert_eq!(w.prints, ["(debug)arrived\n", "path:1,0 \n"]);
        assert!(w.beams.is_empty());
    }

    #[test]
    fn find_path_backtrack_pops_the_current_node() {
        // start 4 - 3 - 1 - goal 0, and a leaf 2 off 1. Dijkstra: g = 4:0, 3:200, 1:400, 2:450.
        // From 1 the walk takes the leaf 2 (g 450, f 450 + 50) over 3 (g 200 but f 200 + 300,
        // not < 500), hits the dead end, pops 2 (itself) and then 1 (re-opened), goes on from
        // 1 to 3 and arrives: the path skips 1, so 3 -> 0 is not a link.
        let nodes = [[0.0, 0.0, 36.0], [100.0, 0.0, 36.0], [50.0, 10.0, 36.0], [300.0, 0.0, 36.0], [500.0, 0.0, 36.0]];
        let mut g = graph(&nodes, &[(0, 1), (1, 2), (1, 3), (3, 4)]);
        let mut w = Fake::default();
        assert!(g.find_path(&mut w, 4, 0, None, true));
        assert_eq!(g.last_path_indices(), [4, 3, 0]);
        assert_eq!(g.last_iterations(), 5);
        assert_eq!(g.link(3, 0).connected, 0);
        assert_eq!(w.prints, ["(debug)arrived\n", "path:4,3,0 \n"]);
    }

    // ---- AddNode and linking ------------------------------------------------------------------

    #[test]
    fn ground_spacing_and_truncated_distances() {
        let mut g = NodeGraph::new();
        let mut w = Fake::default();
        assert!(g.add_node(&mut w, [0.0, 0.0, 36.0], false, false, false));
        // The in-air probe: 72 straight down, ignoring nothing.
        assert_eq!(w.traces, [([0.0, 0.0, 36.0], [0.0, 0.0, -36.0], std::ptr::null_mut())]);
        assert!(!g.add_node(&mut w, [127.9, 0.0, 36.0], false, false, false), "closer than 128");
        assert!(g.add_node(&mut w, [128.0, 0.0, 36.0], false, false, false), "the spacing test is strict");
        assert!(g.add_node(&mut w, [100.0, 140.0, 36.0], false, false, false));
        assert!(g.add_node(&mut w, [230.9, 140.0, 60.0], false, false, false));
        assert_eq!(g.count(), 4);
        assert_eq!(*g.link(1, 0), Link { dist: 128, connected: 1, pad: [0; 3] });
        assert_eq!(g.link(0, 2).dist, 172, "sqrt(100² + 140²) = 172.05, truncated");
        assert_eq!(g.link(3, 2).dist, 133, "sqrt(130.9² + 24²) = 133.08, truncated");
        for i in 0..4 {
            assert_eq!(g.link(i, i).connected, 0);
            for j in 0..4 {
                assert_eq!(g.link(i, j), g.link(j, i));
                assert_eq!(g.link(i, j).connected, (i != j) as u8);
            }
        }
        assert!(w.beams.is_empty() && w.prints.is_empty(), "no debug output without botlearn 2");
    }

    #[test]
    fn ladder_and_manual_spacing() {
        let mut g = NodeGraph::new();
        let mut w = Fake::default();
        assert!(g.add_node(&mut w, [0.0, 0.0, 100.0], true, false, false));
        assert!(g.add_node(&mut w, [0.0, 0.0, 132.0], true, false, false), "32 from a ladder node is far enough");
        assert!(!g.add_node(&mut w, [0.0, 0.0, 163.9], true, false, false));
        // The radius depends on the new node only: a ground node needs 128 even from ladder nodes.
        assert!(!g.add_node(&mut w, [100.0, 0.0, 36.0], false, false, false));
        assert!(g.add_node(&mut w, [8.0, 0.0, 36.0], false, true, false), "manual nodes need 8");
        assert!(!g.add_node(&mut w, [12.0, 0.0, 36.0], false, true, false));
        assert_eq!(g.count(), 3);
        assert_eq!(g.nodes().iter().map(|n| n.flags).collect::<Vec<_>>(), [1, 1, 0]);
        // Manual: links to both ladder nodes (2D < 64), a node beam and the count, debug or not.
        assert_eq!(g.link(2, 0).connected & g.link(2, 1).connected, 1);
        assert_eq!(
            w.beams,
            [
                ([8.0, 0.0, 36.0], [0.0, 0.0, 100.0], 10),
                ([8.0, 0.0, 36.0], [0.0, 0.0, 132.0], 10),
                ([8.0, 0.0, 52.0], [8.0, 0.0, 20.0], 10)
            ]
        );
        assert_eq!(w.prints, ["(debug)3 nodes\n"]);
    }

    #[test]
    fn in_air_gaps_and_ladder_shortcuts() {
        let mut w = Fake { holes: vec![[400.0, 600.0, -1000.0, 1000.0]], ..Default::default() };
        let mut g = NodeGraph::new();
        assert!(!g.add_node(&mut w, [500.0, 0.0, 36.0], false, false, false), "no floor within 72 below");
        assert!(!g.add_node(&mut w, [0.0, 0.0, 109.0], false, false, false), "floor 109 below");
        assert!(g.add_node(&mut w, [300.0, 0.0, 36.0], false, false, false)); // 0
        assert!(g.add_node(&mut w, [700.0, 400.0, 36.0], false, false, false)); // 1
        assert_eq!(g.link(0, 1).connected, 0, "in sight but the floor has a gap");
        assert!(g.add_node(&mut w, [500.0, 0.0, 36.0], true, false, false), "ladder nodes may hang in the air"); // 2
        assert_eq!(g.link(2, 0).connected + g.link(2, 1).connected, 0, "2D 200 > 160: the ground test fails");
        assert!(g.add_node(&mut w, [650.0, -50.0, 50.0], false, false, false)); // 3
        assert_eq!(g.link(3, 2).connected, 1, "2D 158 < 160 and dz 14 < 32");
        assert!(g.add_node(&mut w, [610.0, 100.0, 70.0], false, false, false)); // 4
        assert_eq!(g.link(4, 2).connected, 0, "2D 149 < 160 but dz 34: the ground test fails");
        assert_eq!(g.link(4, 3).connected, 1);
        assert!(g.add_node(&mut w, [500.0, 60.0, 300.0], true, false, false)); // 5
        assert_eq!(g.link(5, 2).connected, 1, "2D 60 < 64");
        assert_eq!((0..5).filter(|&j| g.link(5, j).connected != 0).count(), 1);
    }

    #[test]
    fn ground_probes_every_16_units() {
        let mut g = NodeGraph::new();
        let mut w = Fake::default();
        g.add_node(&mut w, [0.0, 0.0, 36.0], false, false, false);
        w.traces.clear();
        g.add_node(&mut w, [0.0, 130.0, 36.0], false, false, false);
        // In-air probe, the line of sight, then __ftol(130 / 16) = 8 probes from the new node.
        assert_eq!(w.traces.len(), 2 + 8);
        assert_eq!(w.traces[1], ([0.0, 130.0, 36.0], [0.0, 0.0, 36.0], std::ptr::null_mut()));
        let step = -130.0f32 / 8.0;
        for (k, t) in w.traces[2..].iter().enumerate() {
            let y = step * (k + 1) as f32 + 130.0;
            assert_eq!(*t, ([0.0, y, 36.0], [0.0, y, -36.0], std::ptr::null_mut()));
        }
    }

    #[test]
    fn walls_block_and_doors_are_traced_through() {
        let mut w = Fake { walls: vec![(64.0, -100.0, 100.0)], ..Default::default() };
        let mut g = NodeGraph::new();
        g.add_node(&mut w, [0.0, 0.0, 36.0], false, false, false);
        g.add_node(&mut w, [128.0, 0.0, 36.0], false, false, false);
        assert_eq!(g.link(0, 1).connected, 0);

        let mut w = Fake { doors: vec![64.0], ..Default::default() };
        let mut g = NodeGraph::new();
        g.add_node(&mut w, [0.0, 0.0, 36.0], false, false, false);
        w.traces.clear();
        g.add_node(&mut w, [128.0, 0.0, 36.0], false, true, false);
        assert_eq!(g.link(0, 1).connected, 1);
        // From the door's hit point on, ignoring the door.
        assert_eq!(w.traces[1], ([128.0, 0.0, 36.0], [0.0, 0.0, 36.0], std::ptr::null_mut()));
        assert_eq!(w.traces[2], ([64.0, 0.0, 36.0], [0.0, 0.0, 36.0], door(0)));
        // The link beam uses the node origins.
        assert_eq!(w.beams[0], ([128.0, 0.0, 36.0], [0.0, 0.0, 36.0], 10));

        // Two doors in the same place: TS would trace forever; the port gives up unlinked.
        let mut w = Fake { doors: vec![64.0, 64.0], ..Default::default() };
        let mut g = NodeGraph::new();
        g.add_node(&mut w, [0.0, 0.0, 36.0], false, false, false);
        w.traces.clear();
        g.add_node(&mut w, [128.0, 0.0, 36.0], false, false, false);
        assert_eq!(g.link(0, 1).connected, 0);
        assert_eq!(w.traces.len(), 1 + 1 + DOOR_TRACE_LIMIT);
    }

    #[test]
    fn graph_full_at_512_and_the_file_load_refuses() {
        let mut w = Fake { opaque: true, ..Default::default() };
        let mut g = NodeGraph::new();
        let grid = (0..23).flat_map(|x| (0..23).map(move |y| [x as f32 * 200.0, y as f32 * 200.0, 36.0]));
        let added = grid.map(|o| g.add_node(&mut w, o, false, false, true)).filter(|&a| a).count();
        assert_eq!((added, g.count()), (512, 512));
        assert_eq!(w.prints.last().unwrap(), "(debug)512 nodes\n");
        let file = g.save_bytes().unwrap();
        assert_eq!(file.len(), 4 + 20 * 512 + 8 * 512 * 512);
        g.load_bytes(Some(&file));
        assert_eq!(g.count(), 0);
    }

    // ---- FindPathBetweenPoints ----------------------------------------------------------------

    #[test]
    fn between_points_takes_the_nearest_visible_nodes() {
        let nodes = [[0.0, 0.0, 36.0], [256.0, 0.0, 36.0], [512.0, 0.0, 36.0]];
        let mut g = graph(&nodes, &[(0, 1), (1, 2)]);
        let mut w = Fake::default();
        let mut out = Vec::new();
        // 128 from both 0 and 1: the lower index wins. `to` is nearest to 2.
        let me = door(7);
        assert!(g.find_path_between_points(&mut w, [128.0, 0.0, 36.0], [600.0, 0.0, 36.0], &mut out, me, false));
        assert_eq!(g.last_path_indices(), [0, 1, 2]);
        assert_eq!(out, nodes);
        assert!(w.traces.iter().all(|t| t.2 == me));
        assert_eq!(w.traces.len(), 6);

        // A wall hides node 1 from `from`: node 0 (farther) is taken.
        let mut w = Fake { walls: vec![(200.0, -10.0, 10.0)], ..Default::default() };
        assert!(g.find_path_between_points(&mut w, [190.0, 0.0, 36.0], [600.0, 0.0, 36.0], &mut out, std::ptr::null_mut(), false));
        assert_eq!(g.last_path_indices(), [0, 1, 2]);

        // Both ends nearest to the same node: FindPath(i, i) fails.
        assert!(!g.find_path_between_points(&mut w, [500.0, 0.0, 36.0], [520.0, 0.0, 36.0], &mut out, std::ptr::null_mut(), false));
        // 9999 units or more never count: no node, FindPath(-1, ...) fails and `out` is kept.
        let mut out = vec![[3.0; 3]];
        assert!(!g.find_path_between_points(&mut w, [10511.0, 0.0, 36.0], [0.0, 0.0, 36.0], &mut out, std::ptr::null_mut(), true));
        assert_eq!(out, [[3.0; 3]]);
        assert!(w.prints.is_empty());
        assert!(g.find_path_between_points(&mut w, [10510.0, 0.0, 36.0], [0.0, 0.0, 36.0], &mut out, std::ptr::null_mut(), false));
        assert_eq!(g.last_path_indices(), [2, 1, 0]);
    }

    #[test]
    fn atoi_is_c_atoi() {
        for (s, v) in [("12", 12), ("  -7x", -7), ("+3", 3), ("abc", 0), ("", 0), ("4294967297", 1), ("- 1", 0)] {
            assert_eq!(atoi(s), v, "{s:?}");
        }
    }
}
