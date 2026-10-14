//! Android's logical partition metadata in `super` (AOSP's liblp, metadata 10.x): read, checked, changed, written
//! (ADR-0067). On the R1 (Max's, 2026-10-14): geometry at 4096 and its backup at 8192; then three metadata slots of
//! 64 KiB, then their backups; header 10.2 (256 bytes, the Virtual A/B flag); logical partitions from sector 2048 of
//! one block device, `super`, 8,792,064,000 bytes.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

pub const SECTOR: u64 = 512;
pub const GEOMETRY_MAGIC: u32 = 0x616c_4467;
pub const HEADER_MAGIC: u32 = 0x414c_5030;
/// Bytes before the geometry, reserved (never written).
pub const RESERVED: u64 = 4096;
/// The geometry's space; its backup follows.
pub const GEOMETRY_SIZE: u64 = 4096;
/// Where the metadata slots start: after the reserved bytes and both geometry copies.
pub const METADATA_OFFSET: u64 = RESERVED + 2 * GEOMETRY_SIZE;
pub const ATTR_READONLY: u32 = 1;
pub const FLAG_VIRTUAL_AB: u32 = 1;
pub const TARGET_LINEAR: u32 = 0;

const GEOMETRY_STRUCT: usize = 52;
const PARTITION_SIZE: u32 = 52;
const EXTENT_SIZE: u32 = 24;
const GROUP_SIZE: u32 = 48;
const BLOCK_DEVICE_SIZE: u32 = 64;
const NAME_LEN: usize = 36;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Geometry {
    pub metadata_max_size: u32,
    pub metadata_slot_count: u32,
    pub logical_block_size: u32,
}

impl Geometry {
    /// The bytes the metadata slots and their backups take, from [METADATA_OFFSET].
    pub fn region_len(&self) -> u64 {
        2 * self.metadata_max_size as u64 * self.metadata_slot_count as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extent {
    pub sectors: u64,
    pub target_type: u32,
    /// The first sector on the block device, for a linear extent.
    pub target_data: u64,
    pub target_source: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub name: String,
    pub attributes: u32,
    /// An index into [Metadata::groups].
    pub group: u32,
    pub extents: Vec<Extent>,
}

impl Partition {
    pub fn sectors(&self) -> u64 {
        self.extents.iter().map(|e| e.sectors).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub flags: u32,
    /// 0: no limit.
    pub maximum_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDevice {
    pub first_logical_sector: u64,
    pub alignment: u32,
    pub alignment_offset: u32,
    pub size: u64,
    pub name: String,
    pub flags: u32,
}

/// One slot's metadata, with the geometry it sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub geometry: Geometry,
    pub major: u16,
    pub minor: u16,
    pub header_size: u32,
    pub flags: u32,
    pub partitions: Vec<Partition>,
    pub groups: Vec<Group>,
    pub block_devices: Vec<BlockDevice>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn name_at(b: &[u8], o: usize) -> Result<String> {
    let raw = &b[o..o + NAME_LEN];
    let end = raw.iter().position(|&c| c == 0).unwrap_or(NAME_LEN);
    let s = std::str::from_utf8(&raw[..end]).context("a name that isn't text")?;
    Ok(s.to_owned())
}
fn put_name(out: &mut Vec<u8>, name: &str) -> Result<()> {
    if name.len() >= NAME_LEN || !name.bytes().all(|c| c.is_ascii_graphic()) {
        bail!("{name:?} can't be a partition, group or device name");
    }
    let mut b = [0u8; NAME_LEN];
    b[..name.len()].copy_from_slice(name.as_bytes());
    out.extend_from_slice(&b);
    Ok(())
}
fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// The geometry at [RESERVED] of [head] (`super`'s first bytes), checked; its backup must agree.
pub fn geometry(head: &[u8]) -> Result<Geometry> {
    let at = |o: u64| -> Result<Geometry> {
        let b = head.get(o as usize..o as usize + GEOMETRY_STRUCT).context("too little of super to hold its geometry")?;
        if u32_at(b, 0) != GEOMETRY_MAGIC || u32_at(b, 4) as usize != GEOMETRY_STRUCT {
            bail!("no LP geometry at {o}");
        }
        let mut zeroed = b.to_vec();
        zeroed[8..40].fill(0);
        if sha256(&[&zeroed]) != b[8..40] {
            bail!("the LP geometry's checksum doesn't match");
        }
        Ok(Geometry { metadata_max_size: u32_at(b, 40), metadata_slot_count: u32_at(b, 44), logical_block_size: u32_at(b, 48) })
    };
    let g = at(RESERVED)?;
    if at(RESERVED + GEOMETRY_SIZE)? != g {
        bail!("the LP geometry and its backup differ");
    }
    if g.metadata_max_size == 0 || g.metadata_max_size % 512 != 0 || !(1..=3).contains(&g.metadata_slot_count) || g.logical_block_size % 512 != 0 {
        bail!("an LP geometry this doesn't know: {g:?}");
    }
    Ok(g)
}

/// Metadata slot [slot] from [head] (`super`'s first bytes), checked; its backup must agree.
pub fn read(head: &[u8], slot: u32) -> Result<Metadata> {
    let geometry = geometry(head)?;
    if slot >= geometry.metadata_slot_count {
        bail!("no metadata slot {slot}");
    }
    let max = geometry.metadata_max_size as u64;
    let primary = METADATA_OFFSET + slot as u64 * max;
    let backup = METADATA_OFFSET + (geometry.metadata_slot_count as u64 + slot as u64) * max;
    let p = head.get(primary as usize..(primary + max) as usize).context("too little of super to hold its metadata")?;
    let b = head.get(backup as usize..(backup + max) as usize).context("too little of super to hold its metadata's backup")?;
    let m = parse(p, &geometry)?;
    if parse(b, &geometry)? != m {
        bail!("metadata slot {slot} and its backup differ");
    }
    Ok(m)
}

fn parse(b: &[u8], geometry: &Geometry) -> Result<Metadata> {
    if b.len() < 128 || u32_at(b, 0) != HEADER_MAGIC {
        bail!("no LP metadata header");
    }
    let (major, minor, header_size) = (u16_at(b, 4), u16_at(b, 6), u32_at(b, 8));
    if major != 10 || minor > 2 || !(header_size == 128 || header_size == 256) || b.len() < header_size as usize {
        bail!("LP metadata {major}.{minor} (header {header_size} bytes) isn't one this knows");
    }
    let header = &b[..header_size as usize];
    let mut zeroed = header.to_vec();
    zeroed[12..44].fill(0);
    if sha256(&[&zeroed]) != header[12..44] {
        bail!("the LP header's checksum doesn't match");
    }
    let tables_size = u32_at(b, 44) as usize;
    let tables = b.get(header_size as usize..header_size as usize + tables_size).context("LP tables past the slot's end")?;
    if sha256(&[tables]) != header[48..80] {
        bail!("the LP tables' checksum doesn't match");
    }
    let desc = |i: usize| (u32_at(b, 80 + 12 * i), u32_at(b, 84 + 12 * i), u32_at(b, 88 + 12 * i));
    let table = |i: usize, size: u32| -> Result<Vec<&[u8]>> {
        let (off, n, es) = desc(i);
        if es != size {
            bail!("an LP table's entries are {es} bytes, not {size}");
        }
        (0..n as usize)
            .map(|k| tables.get(off as usize + k * es as usize..off as usize + (k + 1) * es as usize).context("an LP table past its end"))
            .collect()
    };
    let extents: Vec<Extent> = table(1, EXTENT_SIZE)?
        .into_iter()
        .map(|e| Extent { sectors: u64_at(e, 0), target_type: u32_at(e, 8), target_data: u64_at(e, 12), target_source: u32_at(e, 20) })
        .collect();
    let groups = table(2, GROUP_SIZE)?
        .into_iter()
        .map(|g| Ok(Group { name: name_at(g, 0)?, flags: u32_at(g, 36), maximum_size: u64_at(g, 40) }))
        .collect::<Result<Vec<_>>>()?;
    let block_devices = table(3, BLOCK_DEVICE_SIZE)?
        .into_iter()
        .map(|d| {
            Ok(BlockDevice {
                first_logical_sector: u64_at(d, 0),
                alignment: u32_at(d, 8),
                alignment_offset: u32_at(d, 12),
                size: u64_at(d, 16),
                name: name_at(d, 24)?,
                flags: u32_at(d, 60),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let partitions = table(0, PARTITION_SIZE)?
        .into_iter()
        .map(|p| {
            let (first, n, group) = (u32_at(p, 40) as usize, u32_at(p, 44) as usize, u32_at(p, 48));
            if group as usize >= groups.len() {
                bail!("a partition in a group that isn't there");
            }
            let ex = extents.get(first..first + n).context("a partition's extents past the table's end")?.to_vec();
            Ok(Partition { name: name_at(p, 0)?, attributes: u32_at(p, 36), group, extents: ex })
        })
        .collect::<Result<Vec<_>>>()?;
    let flags = if header_size >= 132 { u32_at(b, 128) } else { 0 };
    Ok(Metadata { geometry: geometry.clone(), major, minor, header_size, flags, partitions, groups, block_devices })
}

impl Metadata {
    pub fn partition(&self, name: &str) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.name == name)
    }

    pub fn group_index(&self, name: &str) -> Option<u32> {
        self.groups.iter().position(|g| g.name == name).map(|i| i as u32)
    }

    /// One slot's bytes: the header, then the partitions, extents, groups and block devices, with both checksums.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut parts = Vec::new();
        let mut extents = Vec::new();
        for p in &self.partitions {
            put_name(&mut parts, &p.name)?;
            parts.extend_from_slice(&p.attributes.to_le_bytes());
            parts.extend_from_slice(&((extents.len() / EXTENT_SIZE as usize) as u32).to_le_bytes());
            parts.extend_from_slice(&(p.extents.len() as u32).to_le_bytes());
            parts.extend_from_slice(&p.group.to_le_bytes());
            for e in &p.extents {
                extents.extend_from_slice(&e.sectors.to_le_bytes());
                extents.extend_from_slice(&e.target_type.to_le_bytes());
                extents.extend_from_slice(&e.target_data.to_le_bytes());
                extents.extend_from_slice(&e.target_source.to_le_bytes());
            }
        }
        let mut groups = Vec::new();
        for g in &self.groups {
            put_name(&mut groups, &g.name)?;
            groups.extend_from_slice(&g.flags.to_le_bytes());
            groups.extend_from_slice(&g.maximum_size.to_le_bytes());
        }
        let mut devices = Vec::new();
        for d in &self.block_devices {
            devices.extend_from_slice(&d.first_logical_sector.to_le_bytes());
            devices.extend_from_slice(&d.alignment.to_le_bytes());
            devices.extend_from_slice(&d.alignment_offset.to_le_bytes());
            devices.extend_from_slice(&d.size.to_le_bytes());
            put_name(&mut devices, &d.name)?;
            devices.extend_from_slice(&d.flags.to_le_bytes());
        }
        let tables: Vec<u8> = [&parts[..], &extents, &groups, &devices].concat();
        let mut h = vec![0u8; self.header_size as usize];
        h[0..4].copy_from_slice(&HEADER_MAGIC.to_le_bytes());
        h[4..6].copy_from_slice(&self.major.to_le_bytes());
        h[6..8].copy_from_slice(&self.minor.to_le_bytes());
        h[8..12].copy_from_slice(&self.header_size.to_le_bytes());
        h[44..48].copy_from_slice(&(tables.len() as u32).to_le_bytes());
        h[48..80].copy_from_slice(&sha256(&[&tables]));
        let mut off = 0u32;
        for (i, (len, n, es)) in [
            (parts.len(), self.partitions.len(), PARTITION_SIZE),
            (extents.len(), extents.len() / EXTENT_SIZE as usize, EXTENT_SIZE),
            (groups.len(), self.groups.len(), GROUP_SIZE),
            (devices.len(), self.block_devices.len(), BLOCK_DEVICE_SIZE),
        ]
        .into_iter()
        .enumerate()
        {
            h[80 + 12 * i..84 + 12 * i].copy_from_slice(&off.to_le_bytes());
            h[84 + 12 * i..88 + 12 * i].copy_from_slice(&(n as u32).to_le_bytes());
            h[88 + 12 * i..92 + 12 * i].copy_from_slice(&es.to_le_bytes());
            off += len as u32;
        }
        if self.header_size >= 132 {
            h[128..132].copy_from_slice(&self.flags.to_le_bytes());
        }
        let sum = sha256(&[&h]);
        h[12..44].copy_from_slice(&sum);
        let out = [h, tables].concat();
        if out.len() as u64 > self.geometry.metadata_max_size as u64 {
            bail!("the metadata ({} bytes) doesn't fit its slot ({})", out.len(), self.geometry.metadata_max_size);
        }
        Ok(out)
    }

    /// Every slot and its backup, each this metadata padded to the slot's size: the bytes from [METADATA_OFFSET].
    pub fn region(&self) -> Result<Vec<u8>> {
        let mut one = self.to_bytes()?;
        one.resize(self.geometry.metadata_max_size as usize, 0);
        Ok(one.repeat(2 * self.geometry.metadata_slot_count as usize))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `super`'s first bytes like the R1's (2026-10-14): system and vendor on both slots, product gone.
    pub(crate) fn r1_like() -> (Vec<u8>, Metadata) {
        let geometry = Geometry { metadata_max_size: 65536, metadata_slot_count: 3, logical_block_size: 4096 };
        let lin = |sectors: u64, at: u64| Extent { sectors, target_type: TARGET_LINEAR, target_data: at, target_source: 0 };
        let m = Metadata {
            geometry: geometry.clone(),
            major: 10,
            minor: 2,
            header_size: 256,
            flags: FLAG_VIRTUAL_AB,
            partitions: vec![
                Partition { name: "system_a".into(), attributes: ATTR_READONLY, group: 1, extents: vec![lin(3_095_704, 368_640), lin(366_512, 2048), lin(391_288, 8_914_344)] },
                Partition { name: "system_b".into(), attributes: ATTR_READONLY, group: 2, extents: vec![lin(80, 368_560), lin(872, 3_464_344), lin(3_852_552, 4_156_952)] },
                Partition { name: "vendor_a".into(), attributes: ATTR_READONLY, group: 1, extents: vec![lin(691_736, 3_465_216)] },
                Partition { name: "vendor_b".into(), attributes: ATTR_READONLY, group: 2, extents: vec![lin(691_736, 10_210_472)] },
            ],
            groups: vec![
                Group { name: "default".into(), flags: 0, maximum_size: 0 },
                Group { name: "main_a".into(), flags: 0, maximum_size: 8_789_966_848 },
                Group { name: "main_b".into(), flags: 0, maximum_size: 8_789_966_848 },
            ],
            block_devices: vec![BlockDevice { first_logical_sector: 2048, alignment: 512, alignment_offset: 0, size: 8_792_064_000, name: "super".into(), flags: 0 }],
        };
        (head_of(&m), m)
    }

    /// `super`'s first MiB holding [m]: its geometry (and backup), then [m] in every slot and backup.
    pub(crate) fn head_of(m: &Metadata) -> Vec<u8> {
        let geometry = &m.geometry;
        let mut head = vec![0u8; 1 << 20];
        let mut g = vec![0u8; GEOMETRY_STRUCT];
        g[0..4].copy_from_slice(&GEOMETRY_MAGIC.to_le_bytes());
        g[4..8].copy_from_slice(&(GEOMETRY_STRUCT as u32).to_le_bytes());
        g[40..44].copy_from_slice(&geometry.metadata_max_size.to_le_bytes());
        g[44..48].copy_from_slice(&geometry.metadata_slot_count.to_le_bytes());
        g[48..52].copy_from_slice(&geometry.logical_block_size.to_le_bytes());
        let sum = sha256(&[&g]);
        g[8..40].copy_from_slice(&sum);
        head[4096..4096 + 52].copy_from_slice(&g);
        head[8192..8192 + 52].copy_from_slice(&g);
        let region = m.region().unwrap();
        head[METADATA_OFFSET as usize..METADATA_OFFSET as usize + region.len()].copy_from_slice(&region);
        head
    }

    #[test]
    fn written_then_read_back() {
        let (head, m) = r1_like();
        for slot in 0..3 {
            assert_eq!(read(&head, slot).unwrap(), m);
        }
        assert_eq!(m.to_bytes().unwrap().len(), 256 + 608, "the R1's tables are 608 bytes");
        assert_eq!(m.partition("vendor_b").unwrap().sectors(), 691_736);
    }

    #[test]
    fn damage_is_refused() {
        let (head, _) = r1_like();
        // A changed byte in slot 0's tables, in its backup, in the geometry.
        for at in [METADATA_OFFSET as usize + 300, METADATA_OFFSET as usize + 3 * 65536 + 300, 4096 + 44] {
            let mut h = head.clone();
            h[at] ^= 1;
            assert!(read(&h, 0).is_err(), "byte {at}");
        }
        assert!(read(&head[..20000], 0).is_err());
        assert!(read(&head, 3).is_err());
    }
}
