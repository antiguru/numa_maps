#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Properties for memory regions. Consult `man 7 numa` for details.
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum Property {
    /// File backing the memory region.
    File(PathBuf),
    /// Size on each numa node `(numa_node, size)`. Size in pages, or bytes after normalizing.
    N(usize, usize),
    /// Memory range is used for the heap.
    Heap,
    /// Memory range is used for the stack.
    Stack,
    /// Memory range backed by huge pages. Page size differs from other entries.
    Huge,
    /// Number of anonymous pages in range.
    Anon(usize),
    /// Number of dirty pages in range.
    Dirty(usize),
    /// Number of mapped pages in range.
    Mapped(usize),
    /// Number of processes mapping a single page.
    MapMax(usize),
    /// Number of pages that have an associated entry on a swap device.
    SwapCache(usize),
    /// Number of pages on the active list.
    Active(usize),
    /// Number of pages that are currently being written out to disk.
    Writeback(usize),
    /// The size of the pages in this region in bytes.
    Kernelpagesize(usize),
}

impl Property {
    /// Returns the kernel page size if the property matches the page size property.
    #[must_use]
    pub fn page_size(&self) -> Option<usize> {
        match self {
            Self::Kernelpagesize(page_size) => Some(*page_size),
            _ => None,
        }
    }

    /// Normalize the property given the page size. Returns an
    /// optional value, which is set for all but the page size property.
    #[must_use]
    pub fn normalize(self, page_size: usize) -> Option<Self> {
        use Property::*;
        match self {
            File(p) => Some(File(p)),
            N(node, pages) => Some(N(node, pages * page_size)),
            Heap => Some(Heap),
            Stack => Some(Stack),
            Huge => Some(Huge),
            Anon(pages) => Some(Anon(pages * page_size)),
            Dirty(pages) => Some(Dirty(pages * page_size)),
            Mapped(pages) => Some(Mapped(pages * page_size)),
            MapMax(pages) => Some(MapMax(pages)),
            SwapCache(pages) => Some(SwapCache(pages * page_size)),
            Active(pages) => Some(Active(pages * page_size)),
            Writeback(pages) => Some(Writeback(pages * page_size)),
            Kernelpagesize(_) => None,
        }
    }
}

impl FromStr for Property {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (key, val) = if let Some(index) = s.find('=') {
            let (key, val) = s.split_at(index);
            (key, Some(&val[1..]))
        } else {
            (s, None)
        };
        match (key, val) {
            (key, Some(val)) if key.starts_with('N') => {
                let node = key[1..].parse().map_err(|e| format!("{e}"))?;
                let count = val.parse().map_err(|e| format!("{e}"))?;
                Ok(Self::N(node, count))
            }
            ("file", Some(val)) => Ok(Self::File(PathBuf::from(val))),
            ("heap", _) => Ok(Self::Heap),
            ("stack", _) => Ok(Self::Stack),
            ("huge", _) => Ok(Self::Huge),
            ("anon", Some(val)) => val.parse().map(Self::Anon).map_err(|e| format!("{e}")),
            ("dirty", Some(val)) => val.parse().map(Self::Dirty).map_err(|e| format!("{e}")),
            ("mapped", Some(val)) => val.parse().map(Self::Mapped).map_err(|e| format!("{e}")),
            ("mapmax", Some(val)) => val.parse().map(Self::MapMax).map_err(|e| format!("{e}")),
            ("swapcache", Some(val)) => {
                val.parse().map(Self::SwapCache).map_err(|e| format!("{e}"))
            }
            ("active", Some(val)) => val.parse().map(Self::Active).map_err(|e| format!("{e}")),
            ("writeback", Some(val)) => {
                val.parse().map(Self::Writeback).map_err(|e| format!("{e}"))
            }
            ("kernelpagesize_kB", Some(val)) => val
                .parse()
                .map(|sz: usize| Self::Kernelpagesize(sz << 10))
                .map_err(|e| format!("{e}")),
            (key, None) => Err(format!("unknown key: {key}")),
            (key, Some(val)) => Err(format!("unknown key/value: {key}={val}")),
        }
    }
}

/// A memory range, with a base address and a list of properties.
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct Range {
    /// The base address of this memory range.
    pub address: usize,
    /// The range's memory policy.
    pub policy: String,
    /// Properties associated with the memory range.
    pub properties: Vec<Property>,
}

impl Range {
    /// Parse a numa map line. Prints errors to stderr.
    ///
    /// Returns no value if the line does not contain an address, or the address is
    /// malformed.
    fn parse(line: &str) -> Option<Self> {
        let mut parts = line.split_whitespace();
        let address = <usize>::from_str_radix(parts.next()?, 16).ok()?;
        let policy = parts.next()?.to_string();
        let mut properties = Vec::new();
        for part in parts {
            match part.parse::<Property>() {
                Ok(property) => properties.push(property),
                Err(err) => eprintln!("Failed to parse numa_map entry \"{part}\": {err}"),
            }
        }
        Some(Self {
            address,
            policy,
            properties,
        })
    }

    /// Normalize the range using the page size property, if it exists.
    pub fn normalize(&mut self) {
        let page_size = self.properties.iter().find_map(Property::page_size);
        if let Some(page_size) = page_size {
            let mut properties: Vec<_> = self
                .properties
                .drain(..)
                .filter_map(|p| Property::normalize(p, page_size))
                .collect();
            properties.sort();
            self.properties = properties;
        }
    }
}

/// A whole `numu_maps` file.
#[derive(Default)]
pub struct NumaMap {
    /// Individual memory ranges.
    pub ranges: Vec<Range>,
}

impl NumaMap {
    /// Read a `numa_maps` file from `path`.
    ///
    /// Parses the contents and returns them as [`NumaMap`]. Each line translates
    /// to an entry in [`NumaMap::ranges`], which stores the properties gathered
    /// from the file as [`Property`].
    ///
    /// # Errors
    ///
    /// Returns an error if it fails to read the file.
    pub fn from_file<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut ranges = Vec::new();
        for line in reader.lines() {
            if let Some(range) = Range::parse(&(line?)) {
                ranges.push(range);
            }
        }
        Ok(Self { ranges })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_read() -> std::io::Result<()> {
        let map = NumaMap::from_file("resources/numa_maps")?;

        assert_eq!(map.ranges.len(), 23);

        use Property::{Active, Anon, Dirty, File, Heap, Kernelpagesize, MapMax, Mapped, Stack, N};
        let expected = [
            Range {
                address: 93893825802240,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/bin/cat".into()),
                    Mapped(2),
                    MapMax(3),
                    Active(0),
                    N(0, 2),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 93893825810432,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/bin/cat".into()),
                    Mapped(6),
                    MapMax(3),
                    Active(0),
                    N(0, 6),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 93893825835008,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/bin/cat".into()),
                    Mapped(3),
                    MapMax(3),
                    Active(0),
                    N(0, 3),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 93893825847296,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/bin/cat".into()),
                    Anon(1),
                    Dirty(1),
                    Active(0),
                    N(0, 1),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 93893825851392,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/bin/cat".into()),
                    Anon(1),
                    Dirty(1),
                    Active(0),
                    N(0, 1),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 93893836886016,
                policy: "default".to_string(),
                properties: vec![Heap, Anon(2), Dirty(2), N(0, 2), Kernelpagesize(4096)],
            },
            Range {
                address: 140172716933120,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/locale/locale-archive".into()),
                    Mapped(94),
                    MapMax(138),
                    Active(0),
                    N(0, 94),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172721541120,
                policy: "default".to_string(),
                properties: vec![Anon(3), Dirty(3), Active(1), N(0, 3), Kernelpagesize(4096)],
            },
            Range {
                address: 140172721692672,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/libc.so.6".into()),
                    Mapped(38),
                    MapMax(174),
                    N(0, 38),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172721848320,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/libc.so.6".into()),
                    Mapped(192),
                    MapMax(185),
                    N(0, 192),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172723245056,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/libc.so.6".into()),
                    Mapped(43),
                    MapMax(181),
                    N(0, 43),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172723589120,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/libc.so.6".into()),
                    Anon(4),
                    Dirty(4),
                    Active(0),
                    N(0, 4),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172723605504,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/libc.so.6".into()),
                    Anon(2),
                    Dirty(2),
                    Active(0),
                    N(0, 2),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172723613696,
                policy: "default".to_string(),
                properties: vec![Anon(5), Dirty(5), Active(1), N(0, 5), Kernelpagesize(4096)],
            },
            Range {
                address: 140172723773440,
                policy: "default".to_string(),
                properties: vec![Anon(1), Dirty(1), Active(0), N(0, 1), Kernelpagesize(4096)],
            },
            Range {
                address: 140172723781632,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2".into()),
                    Mapped(1),
                    MapMax(173),
                    N(0, 1),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172723785728,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2".into()),
                    Mapped(37),
                    MapMax(181),
                    N(0, 37),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172723937280,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2".into()),
                    Mapped(10),
                    MapMax(173),
                    N(0, 10),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172723978240,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2".into()),
                    Anon(2),
                    Dirty(2),
                    Active(0),
                    N(0, 2),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140172723986432,
                policy: "default".to_string(),
                properties: vec![
                    File("/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2".into()),
                    Anon(2),
                    Dirty(2),
                    Active(0),
                    N(0, 2),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140736328499200,
                policy: "default".to_string(),
                properties: vec![
                    Stack,
                    Anon(3),
                    Dirty(3),
                    Active(0),
                    N(0, 3),
                    Kernelpagesize(4096),
                ],
            },
            Range {
                address: 140736330227712,
                policy: "default".to_string(),
                properties: vec![],
            },
            Range {
                address: 140736330244096,
                policy: "default".to_string(),
                properties: vec![],
            },
        ];

        assert_eq!(&expected[..], &map.ranges[..]);

        Ok(())
    }

    #[test]
    fn test_normalize() {
        use Property::*;

        let line = "7fbd0c10f000 default anon=5 dirty=5 active=1 N0=5 kernelpagesize_kB=4";
        let mut range = Range::parse(line).unwrap();
        range.normalize();
        range.properties.sort();
        let expected = vec![
            N(0, 5 << 12),
            Anon(5 << 12),
            Dirty(5 << 12),
            Active(1 << 12),
        ];
        assert_eq!(&expected, &range.properties);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_read_self() -> std::io::Result<()> {
        let map = NumaMap::from_file("/proc/self/numa_maps")?;
        assert!(map.ranges.len() > 0);
        Ok(())
    }
}
