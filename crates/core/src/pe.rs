//! Minimal PE reader: architecture, imported DLL names (normal and delay-load) and
//! exported names. Used to pick the proxy architecture and DLL name for a game.

use crate::XINPUT_DLLS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X86,
    X64,
}

impl Arch {
    pub fn as_str(self) -> &'static str {
        match self {
            Arch::X86 => "x86",
            Arch::X64 => "x64",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "x86" => Some(Arch::X86),
            "x64" => Some(Arch::X64),
            _ => None,
        }
    }

    pub fn bits(self) -> u32 {
        match self {
            Arch::X86 => 32,
            Arch::X64 => 64,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PeInfo {
    pub arch: Arch,
    /// Lowercase DLL names from the import table.
    pub imports: Vec<String>,
    /// Lowercase DLL names from the delay-load import table.
    pub delay_imports: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PeError {
    NotPe,
    UnsupportedMachine(u16),
}

impl std::fmt::Display for PeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeError::NotPe => f.write_str("the file is not a valid Windows executable"),
            PeError::UnsupportedMachine(m) => {
                write!(
                    f,
                    "unsupported CPU architecture (machine 0x{m:04X}); only x86 and x64 games are supported"
                )
            }
        }
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

struct Image<'a> {
    b: &'a [u8],
    arch: Arch,
    image_base: u64,
    dirs: usize,
    dir_count: usize,
    sections: usize,
    section_count: usize,
}

impl<'a> Image<'a> {
    fn open(b: &'a [u8]) -> Result<Self, PeError> {
        let bad = PeError::NotPe;
        if b.get(..2) != Some(b"MZ") {
            return Err(bad);
        }
        let pe = u32_at(b, 0x3C).ok_or(PeError::NotPe)? as usize;
        if b.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(bad);
        }
        let coff = pe + 4;
        let machine = u16_at(b, coff).ok_or(PeError::NotPe)?;
        let section_count = u16_at(b, coff + 2).ok_or(PeError::NotPe)? as usize;
        let opt_size = u16_at(b, coff + 16).ok_or(PeError::NotPe)? as usize;
        let opt = coff + 20;
        let magic = u16_at(b, opt).ok_or(PeError::NotPe)?;
        let (arch, image_base, count_off, dirs) = match (machine, magic) {
            (0x014C, 0x10B) => (Arch::X86, u32_at(b, opt + 28).ok_or(PeError::NotPe)? as u64, 92, 96),
            (0x8664, 0x20B) => {
                let lo = u32_at(b, opt + 24).ok_or(PeError::NotPe)? as u64;
                let hi = u32_at(b, opt + 28).ok_or(PeError::NotPe)? as u64;
                (Arch::X64, lo | hi << 32, 108, 112)
            }
            (0x014C | 0x8664, _) => return Err(bad),
            (m, _) => return Err(PeError::UnsupportedMachine(m)),
        };
        let dir_count = u32_at(b, opt + count_off).ok_or(PeError::NotPe)? as usize;
        Ok(Image {
            b,
            arch,
            image_base,
            dirs: opt + dirs,
            dir_count,
            sections: opt + opt_size,
            section_count,
        })
    }

    fn dir(&self, index: usize) -> Option<(u32, u32)> {
        if index >= self.dir_count {
            return None;
        }
        let o = self.dirs + index * 8;
        let rva = u32_at(self.b, o)?;
        let size = u32_at(self.b, o + 4)?;
        (rva != 0).then_some((rva, size))
    }

    fn offset(&self, rva: u32) -> Option<usize> {
        for i in 0..self.section_count {
            let s = self.sections + i * 40;
            let vsize = u32_at(self.b, s + 8)?;
            let va = u32_at(self.b, s + 12)?;
            let raw_size = u32_at(self.b, s + 16)?;
            let raw = u32_at(self.b, s + 20)?;
            if rva >= va && rva < va + vsize.max(raw_size) {
                return Some((rva - va + raw) as usize);
            }
        }
        None
    }

    fn cstr(&self, rva: u32) -> Option<String> {
        let o = self.offset(rva)?;
        let rest = self.b.get(o..)?;
        let end = rest.iter().take(260).position(|&c| c == 0)?;
        Some(String::from_utf8_lossy(&rest[..end]).to_ascii_lowercase())
    }

    fn imports(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(mut o) = self.dir(1).and_then(|(rva, _)| self.offset(rva)) else {
            return out;
        };
        while let Some(name_rva) = u32_at(self.b, o + 12) {
            if name_rva == 0 || out.len() > 1024 {
                break;
            }
            out.extend(self.cstr(name_rva));
            o += 20;
        }
        out
    }

    fn delay_imports(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(mut o) = self.dir(13).and_then(|(rva, _)| self.offset(rva)) else {
            return out;
        };
        while let (Some(attrs), Some(name)) = (u32_at(self.b, o), u32_at(self.b, o + 4)) {
            if name == 0 || out.len() > 1024 {
                break;
            }
            // Old-style descriptors (attributes 0) store virtual addresses, not RVAs.
            let rva = if attrs & 1 == 0 {
                (name as u64).wrapping_sub(self.image_base) as u32
            } else {
                name
            };
            out.extend(self.cstr(rva));
            o += 32;
        }
        out
    }

    fn exports(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(o) = self.dir(0).and_then(|(rva, _)| self.offset(rva)) else {
            return out;
        };
        let (Some(count), Some(names)) = (u32_at(self.b, o + 24), u32_at(self.b, o + 32)) else {
            return out;
        };
        let Some(names) = self.offset(names) else { return out };
        for i in 0..count.min(4096) as usize {
            if let Some(name) = u32_at(self.b, names + i * 4).and_then(|rva| self.cstr(rva)) {
                out.push(name);
            }
        }
        out
    }
}

pub fn parse(b: &[u8]) -> Result<PeInfo, PeError> {
    let img = Image::open(b)?;
    Ok(PeInfo {
        arch: img.arch,
        imports: img.imports(),
        delay_imports: img.delay_imports(),
    })
}

/// Lowercase exported names (used by tests to compare with the system XInput).
pub fn exports(b: &[u8]) -> Result<Vec<String>, PeError> {
    Ok(Image::open(b)?.exports())
}

/// How the XInput DLL name was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DllSource {
    ImportTable,
    DelayImport,
    /// Found as a string in the file (the game loads it with LoadLibrary).
    StringScan,
    /// Nothing found: most common name used as a guess.
    Default,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Detection {
    pub arch: Arch,
    pub dll: &'static str,
    pub source: DllSource,
    /// Other XInput names seen, which makes the guess ambiguous.
    pub others: Vec<&'static str>,
}

/// Picks the architecture and XInput DLL name for a game executable.
pub fn detect(b: &[u8]) -> Result<Detection, PeError> {
    let info = parse(b)?;
    let pick = |names: &[String]| -> Vec<&'static str> {
        XINPUT_DLLS
            .iter()
            .copied()
            .filter(|d| names.iter().any(|n| n == d))
            .collect()
    };
    let candidates = [
        (DllSource::ImportTable, pick(&info.imports)),
        (DllSource::DelayImport, pick(&info.delay_imports)),
        (DllSource::StringScan, scan_strings(b)),
    ];
    for (source, found) in candidates {
        if let Some((&dll, rest)) = found.split_first() {
            return Ok(Detection {
                arch: info.arch,
                dll,
                source,
                others: rest.to_vec(),
            });
        }
    }
    Ok(Detection {
        arch: info.arch,
        dll: "xinput1_3.dll",
        source: DllSource::Default,
        others: Vec::new(),
    })
}

/// XInput DLL names present anywhere in the file as ASCII or UTF-16LE (case-insensitive),
/// in the order of XINPUT_DLLS (preferred first).
fn scan_strings(b: &[u8]) -> Vec<&'static str> {
    XINPUT_DLLS
        .iter()
        .copied()
        .filter(|name| {
            let ascii = name.as_bytes();
            let wide: Vec<u8> = ascii.iter().flat_map(|&c| [c, 0]).collect();
            contains_ci(b, ascii) || contains_ci(b, &wide)
        })
        .collect()
}

fn contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// True when the file contains the proxy marker, meaning it is our own proxy DLL.
pub fn is_proxy(b: &[u8]) -> bool {
    hay_contains(b, crate::PROXY_MARKER)
}

fn hay_contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a tiny PE image with one section holding the import directory.
    fn fake_pe(machine: u16, imports: &[&str], delay: &[&str]) -> Vec<u8> {
        let pe64 = machine == 0x8664;
        let opt_size: usize = if pe64 { 240 } else { 224 };
        let mut b = vec![0u8; 0x400];
        b[..2].copy_from_slice(b"MZ");
        b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        b[coff..coff + 2].copy_from_slice(&machine.to_le_bytes());
        b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        b[coff + 16..coff + 18].copy_from_slice(&(opt_size as u16).to_le_bytes());
        let opt = coff + 20;
        b[opt..opt + 2].copy_from_slice(&(if pe64 { 0x20Bu16 } else { 0x10B }).to_le_bytes());
        let (count_off, dirs) = if pe64 { (108, 112) } else { (92, 96) };
        b[opt + count_off..opt + count_off + 4].copy_from_slice(&16u32.to_le_bytes());
        // Section: RVA 0x1000 maps to file offset 0x200.
        let sec = opt + opt_size;
        b[sec..sec + 5].copy_from_slice(b".data");
        b[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        b[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        b[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes());
        b[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
        let rva = |off: usize| (off - 0x200 + 0x1000) as u32;
        // Layout inside the section: descriptors at 0x200, delay at 0x280, names at 0x300.
        let mut name_off = 0x300;
        let mut put_name = |b: &mut Vec<u8>, n: &str| {
            b[name_off..name_off + n.len()].copy_from_slice(n.as_bytes());
            let r = rva(name_off);
            name_off += n.len() + 1;
            r
        };
        for (i, n) in imports.iter().enumerate() {
            let r = put_name(&mut b, n);
            let d = 0x200 + i * 20;
            b[d + 12..d + 16].copy_from_slice(&r.to_le_bytes());
        }
        for (i, n) in delay.iter().enumerate() {
            let r = put_name(&mut b, n);
            let d = 0x280 + i * 32;
            b[d..d + 4].copy_from_slice(&1u32.to_le_bytes());
            b[d + 4..d + 8].copy_from_slice(&r.to_le_bytes());
        }
        if !imports.is_empty() {
            b[dirs + opt + 8..dirs + opt + 12].copy_from_slice(&rva(0x200).to_le_bytes());
        }
        if !delay.is_empty() {
            b[dirs + opt + 13 * 8..dirs + opt + 13 * 8 + 4].copy_from_slice(&rva(0x280).to_le_bytes());
        }
        b
    }

    #[test]
    fn detects_imported_xinput_x64() {
        let b = fake_pe(0x8664, &["KERNEL32.dll", "XINPUT1_3.dll"], &[]);
        let d = detect(&b).unwrap();
        assert_eq!(
            (d.arch, d.dll, d.source),
            (Arch::X64, "xinput1_3.dll", DllSource::ImportTable)
        );
    }

    #[test]
    fn detects_delay_import_x86() {
        let b = fake_pe(0x014C, &["kernel32.dll"], &["xinput9_1_0.dll"]);
        let d = detect(&b).unwrap();
        assert_eq!(
            (d.arch, d.dll, d.source),
            (Arch::X86, "xinput9_1_0.dll", DllSource::DelayImport)
        );
    }

    #[test]
    fn falls_back_to_strings_then_default() {
        let mut b = fake_pe(0x8664, &["kernel32.dll"], &[]);
        let d = detect(&b).unwrap();
        assert_eq!((d.dll, d.source), ("xinput1_3.dll", DllSource::Default));
        let wide: Vec<u8> = "XInput1_4.dll".bytes().flat_map(|c| [c, 0]).collect();
        b.extend_from_slice(&wide);
        b.extend_from_slice(b"xinput1_3.dll");
        let d = detect(&b).unwrap();
        assert_eq!(
            (d.dll, d.source, d.others),
            ("xinput1_4.dll", DllSource::StringScan, vec!["xinput1_3.dll"])
        );
    }

    #[test]
    fn rejects_non_pe_and_arm64() {
        assert_eq!(parse(b"hello").unwrap_err(), PeError::NotPe);
        let b = fake_pe(0xAA64, &[], &[]);
        assert_eq!(parse(&b).unwrap_err(), PeError::UnsupportedMachine(0xAA64));
    }

    #[test]
    fn reads_real_system_dll() {
        let Ok(b) = std::fs::read(r"C:\Windows\System32\xinput1_4.dll") else {
            return;
        };
        let ex = exports(&b).unwrap();
        assert!(ex.contains(&"xinputgetstate".to_string()));
        assert_eq!(parse(&b).unwrap().arch, Arch::X64);
        assert!(!is_proxy(&b));
    }
}
