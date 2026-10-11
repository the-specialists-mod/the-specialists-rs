//! `bots.txt` and `bot_say.txt` (bots.md §3), the bot name picker (§2.4) and the chat line
//! formatting (§10.2). Pure code: the loaders take the file bytes, the pickers take the engine's
//! random call as a closure, so everything here is unit tested.
//!
//! TS keeps the tables in fixed `char[64]` slots and fills them with unbounded `strcpy`s, trimming
//! only up to byte 63. The port keeps the same byte layout so the overflow quirks come out the same
//! (the shipped `bot_say.txt` has a freetime line one byte too long, which TS joins with the next
//! line). Guards (TS overruns its tables or the stack there; the port never does): a `bots.txt`
//! line spilling past the last name slot is cut at the end of the skill table (TS runs on into
//! the count and the used flags), and a formatted chat line is cut at 127 bytes.

/// Name slots in `bots.txt` (`g_botNames`, `char[64][64]` at 0x100db1e0).
pub const MAX_NAMES: usize = 64;
/// Bytes per name or chat slot.
const SLOT: usize = 64;
/// `fgets(line, 1024, f)`: at most 1023 bytes per read.
const LINE: usize = 1023;
/// Chat sections: 0 `offense`, 1 `freetime`, 2 `killed`.
pub const OFFENSE: i32 = 0;
pub const FREETIME: i32 = 1;
pub const KILLED: i32 = 2;
/// Stride of the chat table (`char[3][256][64]` at 0x100dc324; only 64 slots per section are used).
const SAY_STRIDE: usize = 256;
const SAY_MAX: usize = 64;

/// C `fgets` over a byte buffer: up to 1023 bytes, through the `\n` if it comes first.
fn fgets(data: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    if *pos >= data.len() {
        return None;
    }
    let rest = &data[*pos..];
    let mut n = rest.iter().position(|&c| c == b'\n').map_or(rest.len(), |i| i + 1);
    n = n.min(LINE);
    *pos += n;
    Some(rest[..n].to_vec())
}

/// MSVC `isspace` for the bytes the trims test (`(int)(signed char)c`): bytes from 0x80 are
/// negative there and never white space in practice.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `strcpy(slot, line)` into `mem` at `at`, then the shared trim (0x1006bee0 / 0x1006bc00): drop
/// trailing white space and zero the slot from there up to byte 63 only. Returns false when the
/// stored string is empty. Bytes past the end of `mem` are dropped (guard).
fn store_trimmed(mem: &mut [u8], at: usize, line: &[u8]) {
    let line = &line[..line.iter().position(|&c| c == 0).unwrap_or(line.len())];
    for (k, &c) in line.iter().chain(std::iter::once(&0)).enumerate() {
        if let Some(b) = mem.get_mut(at + k) {
            *b = c;
        }
    }
    let mut i = line.len() as isize - 1;
    while i >= 0 && is_space(line[i as usize]) {
        i -= 1;
    }
    for k in (i + 1) as usize..SLOT {
        if let Some(b) = mem.get_mut(at + k) {
            *b = 0;
        }
    }
}

/// The C string at `at` (up to its NUL, or the end of `mem`).
fn c_str(mem: &[u8], at: usize) -> &[u8] {
    let s = mem.get(at..).unwrap_or(&[]);
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// MSVC `sscanf(s, "%f")`: leading white space, sign, digits, a point, digits, an exponent.
/// `None` when there is no number (the target is left alone then).
pub fn scan_f32(s: &[u8]) -> Option<f32> {
    let mut i = s.iter().position(|&c| !is_space(c)).unwrap_or(s.len());
    let start = i;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let b = *i;
        while *i < s.len() && s[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - b
    };
    let mut n = digits(&mut i);
    if i < s.len() && s[i] == b'.' {
        i += 1;
        n += digits(&mut i);
    }
    if n == 0 {
        return None;
    }
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        if digits(&mut j) > 0 {
            i = j;
        }
    }
    std::str::from_utf8(&s[start..i]).ok()?.parse::<f64>().ok().map(|v| v as f32)
}

/// The bot name table (`g_botNames`, `g_botNameSkill`, `g_botNameCount`, `g_botNameUsed`).
#[derive(Clone, Debug, PartialEq)]
pub struct BotNames {
    /// The names (64 slots of 64 bytes) followed by the 64 skills as raw `float`s, one block as
    /// in TS, so a line spilling out of the last slot lands in the skills.
    mem: Vec<u8>,
    pub count: usize,
    pub used: [bool; MAX_NAMES],
}

impl Default for BotNames {
    fn default() -> Self {
        BotNames { mem: vec![0; MAX_NAMES * SLOT + MAX_NAMES * 4], count: 0, used: [false; MAX_NAMES] }
    }
}

impl BotNames {
    /// `BotLoadFiles` (0x1006bee0), `bots.txt` part: one name per line, `Name//skill` gives that
    /// name a skill. A blank line isn't counted; a line starting with `//` is an empty name.
    /// At most 64 names; the rest of the file is ignored. `None` (no file) leaves no names.
    pub fn parse(file: Option<&[u8]>) -> BotNames {
        let mut out = BotNames::default();
        let Some(data) = file else { return out };
        let mut pos = 0;
        while let Some(line) = fgets(data, &mut pos) {
            let at = out.count * SLOT;
            store_trimmed(&mut out.mem, at, &line);
            if out.mem[at] == 0 {
                continue;
            }
            let name = c_str(&out.mem, at).to_vec();
            if let Some(c) = name.windows(2).position(|w| w == b"//") {
                if let Some(v) = scan_f32(&name[c + 2..]) {
                    out.set_skill(out.count, v);
                }
                out.mem[at + c] = 0; // strcpy(c, ""): cut the name at "//"
            }
            out.count += 1;
            if out.count >= MAX_NAMES {
                break;
            }
        }
        out
    }

    fn set_skill(&mut self, i: usize, v: f32) {
        let at = MAX_NAMES * SLOT + i * 4;
        self.mem[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// `g_botNameSkill[i]`.
    pub fn skill(&self, i: usize) -> f32 {
        let at = MAX_NAMES * SLOT + i * 4;
        f32::from_le_bytes(self.mem[at..at + 4].try_into().unwrap_or([0; 4]))
    }

    /// Name `i` as TS reads it: the bytes up to the NUL, which may run into the next slots.
    pub fn name(&self, i: usize) -> &[u8] {
        c_str(&self.mem, i * SLOT)
    }

    /// `BotPickName` (0x1006c150): an unused name, searched outward from `RANDOM_LONG(0, count-1)`
    /// (the upward index wins a tie); once every name is used the flags are cleared and it draws
    /// again. "Bot" without names. A positive per-name skill replaces `skill` (no upper clamp).
    pub fn pick(&mut self, mut random_long: impl FnMut(i32, i32) -> i32, skill: &mut f32) -> Vec<u8> {
        let count = self.count as i32;
        if count <= 0 {
            return b"Bot".to_vec();
        }
        loop {
            let mut pick = -1;
            // Guard: an engine RNG outside the range can't index out of the table.
            let mut up = random_long(0, count - 1).clamp(0, count - 1);
            let mut down = up;
            while up < count || down >= 0 {
                if pick != -1 {
                    break;
                }
                if down >= 0 && !self.used[down as usize] {
                    pick = down;
                }
                if up < count && !self.used[up as usize] {
                    pick = up;
                }
                up += 1;
                down -= 1;
            }
            if pick != -1 {
                let i = pick as usize;
                self.used[i] = true;
                if self.skill(i) > 0.0 {
                    *skill = self.skill(i);
                }
                return self.name(i).to_vec();
            }
            self.used = [false; MAX_NAMES];
        }
    }
}

/// The chat table (`g_botSay`, `g_botSayCount[3]`).
#[derive(Clone, Debug, PartialEq)]
pub struct BotSay {
    mem: Vec<u8>,
    pub count: [usize; 3],
}

impl Default for BotSay {
    fn default() -> Self {
        BotSay { mem: vec![0; 3 * SAY_STRIDE * SLOT], count: [0; 3] }
    }
}

impl BotSay {
    /// `BotLoadSay` (0x1006bc00): sections start at a line whose first word is `offense`,
    /// `freetime` or `killed`; lines before the first header are ignored. An empty line keeps the
    /// previous line's first word (`sscanf("%s")` fails and leaves it). Lines keep their leading
    /// white space. Loading stops when any section reaches 64 lines.
    pub fn parse(file: Option<&[u8]>) -> BotSay {
        let mut out = BotSay::default();
        let Some(data) = file else { return out };
        let mut pos = 0;
        let mut sec: i32 = -1;
        let mut word: Vec<u8> = Vec::new();
        while let Some(line) = fgets(data, &mut pos) {
            let text = &line[..line.iter().position(|&c| c == 0).unwrap_or(line.len())];
            let first: Vec<u8> = text.iter().copied().skip_while(|&c| is_space(c)).take_while(|&c| !is_space(c)).collect();
            if !first.is_empty() {
                word = first;
            }
            match word.as_slice() {
                b"offense" => sec = OFFENSE,
                b"freetime" => sec = FREETIME,
                b"killed" => sec = KILLED,
                _ if sec != -1 => {
                    let s = sec as usize;
                    let at = (s * SAY_STRIDE + out.count[s]) * SLOT;
                    store_trimmed(&mut out.mem, at, &line);
                    if out.mem[at] != 0 {
                        out.count[s] += 1;
                        if out.count[s] >= SAY_MAX {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Line `i` of section `sec` as TS reads it.
    pub fn line(&self, sec: usize, i: usize) -> &[u8] {
        c_str(&self.mem, (sec * SAY_STRIDE + i) * SLOT)
    }

    /// `BotSayLine` (0x1006bbc0): a random line of the section (`RANDOM_LONG(0, n-1)`, drawn only
    /// when the section has lines), else `""`.
    pub fn pick(&self, sec: i32, mut random_long: impl FnMut(i32, i32) -> i32) -> Vec<u8> {
        let Some(&n) = usize::try_from(sec).ok().and_then(|s| self.count.get(s)) else { return Vec::new() };
        if n == 0 {
            return Vec::new();
        }
        let i = random_long(0, n as i32 - 1).clamp(0, n as i32 - 1) as usize;
        self.line(sec as usize, i).to_vec()
    }
}

/// `sprintf(buf, line, name)` into `char buf[128]` (the chat module, 0x1006e330): the line is the
/// format and the name its one string argument. `%%` is a percent sign and the first `%s` is the
/// name. Guards: TS reads garbage for any other conversion and for a second `%s`; the port prints
/// nothing for them. The result is cut at 127 bytes (TS overruns the buffer).
pub fn format_line(line: &[u8], name: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut used = false;
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        i += 1;
        if c != b'%' {
            out.push(c);
            continue;
        }
        // %[flags][width][.precision][size]conversion
        while i < line.len() && b"-+ #0".contains(&line[i]) {
            i += 1;
        }
        while i < line.len() && (line[i].is_ascii_digit() || line[i] == b'.' || line[i] == b'*') {
            i += 1;
        }
        while i < line.len() && b"hlLIwqjzt".contains(&line[i]) {
            i += 1;
        }
        let Some(&conv) = line.get(i) else { break };
        i += 1;
        match conv {
            b'%' => out.push(b'%'),
            b's' | b'S' if !used => {
                used = true;
                out.extend_from_slice(name);
            }
            _ => used = true,
        }
    }
    out.truncate(127);
    out
}

/// `BotSay` (0x1006e7a0): `"\x02<name>: <text>\n"` in `char msg[128]`, cut at byte 127 (the
/// unbounded `strcat`s are guarded the same way).
pub fn say_text(netname: &[u8], text: &[u8]) -> Vec<u8> {
    let mut msg = vec![2u8];
    msg.extend_from_slice(netname);
    msg.extend_from_slice(b": ");
    msg.extend_from_slice(text);
    msg.push(b'\n');
    msg.truncate(127);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    #[test]
    fn bots_txt_names_and_skills() {
        let b = BotNames::parse(Some(b"Lombardi\r\n\r\n  \nWinters//0.5\nLe Marc  \nName // 2\n//\n//1.5\n"));
        let names: Vec<String> = (0..b.count).map(|i| s(b.name(i))).collect();
        // A blank line isn't counted; leading white space and the space before `//` are kept;
        // a line starting with `//` is an empty name.
        assert_eq!(names, ["Lombardi", "Winters", "Le Marc", "Name ", "", ""]);
        assert_eq!((0..b.count).map(|i| b.skill(i)).collect::<Vec<_>>(), [0.0, 0.5, 0.0, 2.0, 0.0, 1.5]);
        let many: String = (0..100).map(|i| format!("n{i}\n")).collect();
        assert_eq!(BotNames::parse(Some(many.as_bytes())).count, 64, "TS keeps 64");
        assert_eq!(BotNames::parse(None).count, 0);
        assert_eq!(scan_f32(b" 0.25x"), Some(0.25));
        assert_eq!(scan_f32(b"1e1"), Some(10.0));
        assert_eq!(scan_f32(b"abc"), None);
        assert_eq!(scan_f32(b"."), None);
    }

    #[test]
    fn long_bots_txt_line_spills_into_the_next_slot() {
        // 70 characters: the trim zeroes nothing past byte 63, so "\r\n" stays in slot 1 until
        // the next line overwrites it; the long name then reads on into the next name.
        let long = "A".repeat(70);
        let b = BotNames::parse(Some(format!("{long}\r\nBob\r\n").as_bytes()));
        assert_eq!(b.count, 2);
        assert_eq!(s(b.name(0)), format!("{}Bob", "A".repeat(64)));
        assert_eq!(s(b.name(1)), "Bob");
    }

    #[test]
    fn bot_name_picker() {
        let mut none = BotNames::parse(Some(b""));
        let mut skill = 0.3;
        assert_eq!(none.pick(|_, _| 0, &mut skill), b"Bot");
        let mut b = BotNames::parse(Some(b"a\nb//0.9\nc\n"));
        let mut draws = Vec::new();
        // Start at 1: unused, taken; the file's skill replaces the random one.
        assert_eq!(
            b.pick(
                |lo, hi| {
                    draws.push((lo, hi));
                    1
                },
                &mut skill
            ),
            b"b"
        );
        assert_eq!(draws, [(0, 2)]);
        assert_eq!(skill, 0.9);
        // Start at 1 again: 1 is used; one step out both ways, the upward one wins.
        assert_eq!(b.pick(|_, _| 1, &mut skill), b"c");
        assert_eq!(b.pick(|_, _| 1, &mut skill), b"a");
        // All used: the flags are cleared and it draws again.
        let mut n = 0;
        assert_eq!(
            b.pick(
                |_, _| {
                    n += 1;
                    2
                },
                &mut skill
            ),
            b"c"
        );
        assert_eq!(n, 2, "a second draw after the reset");
        // Out-of-range randoms never index out of bounds.
        assert_eq!(b.pick(|_, _| 99, &mut skill), b"b");
    }

    #[test]
    fn bot_say_sections() {
        let file = b"junk before\r\noffense \r\nYou suck %s\r\n\r\nfreetime\r\nhello\r\n  spaced\r\nkilled\r\nnice shot %s\r\nofFense\r\n";
        let b = BotSay::parse(Some(file));
        // The empty line after an offense line reuses "You" as the word: added as "" and not counted.
        assert_eq!(b.count, [1, 2, 2]);
        assert_eq!(s(b.line(0, 0)), "You suck %s");
        assert_eq!(s(b.line(1, 1)), "  spaced");
        assert_eq!(s(b.line(2, 1)), "ofFense", "headers are case-sensitive");
        assert_eq!(b.pick(5, |_, _| 0), b"");
        assert_eq!(BotSay::default().pick(0, |_, _| panic!("no draw for an empty section")), b"");
    }

    #[test]
    fn bot_say_trim_stops_at_byte_63() {
        // The shipped freetime quirk: 64 visible characters and a space join the next line.
        let first = "I came here to kick ass and chew bubblegum, and I all outta gum! ";
        assert_eq!(first.len(), 65);
        let file = format!("freetime\r\n{first}\r\nKung-Fu only? Anyone?!\r\n");
        let b = BotSay::parse(Some(file.as_bytes()));
        assert_eq!(b.count[1], 2);
        assert_eq!(s(b.line(1, 0)), "I came here to kick ass and chew bubblegum, and I all outta gum!Kung-Fu only? Anyone?!");
    }

    #[test]
    fn shipped_files() {
        let Some(dir) = ["ts_original", "ts_custom"].iter().find_map(|d| crate::bots::testdata_content_dir(d)) else {
            eprintln!("no TS content next to the checkout: skipping the shipped bots.txt/bot_say.txt check");
            return;
        };
        let names = BotNames::parse(std::fs::read(dir.join("bots.txt")).ok().as_deref());
        assert_eq!(names.count, 64, "70 names, only the first 64 load");
        assert_eq!(s(names.name(63)), "Premium");
        let say = BotSay::parse(std::fs::read(dir.join("bot_say.txt")).ok().as_deref());
        assert_eq!(say.count, [25, 29, 42]);
        assert!(s(say.line(1, 0)).ends_with("outta gum!Kung-Fu only? Anyone?!"));
    }

    #[test]
    fn chat_formatting() {
        assert_eq!(format_line(b"You suck %s!", b"Bob"), b"You suck Bob!");
        assert_eq!(format_line(b"100%% %d %s %s", b"Bob"), b"100%   ");
        assert_eq!(format_line(b"plain", b""), b"plain");
        assert_eq!(format_line(&[b'x'; 300], b"").len(), 127);
        assert_eq!(say_text(b"Bob", b"hi"), b"\x02Bob: hi\n");
        assert_eq!(say_text(b"Bob", &[b'x'; 200]).len(), 127);
    }
}
