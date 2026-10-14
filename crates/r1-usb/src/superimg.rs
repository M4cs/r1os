//! ADR-0067's `super`: rewritten in place from the bootloader, so the install never goes through fastbootd.
//!
//! - **[plan]:** new LP metadata from the R1's own. Everything kept stays where it is: the R1's vendor (matched to its
//!   kernel, TEE and modem, whatever Rabbit release it took) is never moved or rewritten. `system_*` and `product_*` go;
//!   r1os's system goes in for both slots, in space nothing kept uses. The other slot gets it only if its own vendor
//!   isn't empty.
//! - **[build]:** a sparse image of `super` that writes only the metadata (every slot and backup) and the system's data.
//!   Everything else is "don't care", left as it is on the R1.
//! - **[verify]:** reads the image back. Every byte it writes is in the metadata region or a new system extent, and
//!   none touches a kept partition; the metadata it writes reads back as the plan. [build] runs it before handing the
//!   image over, and only an image that passed can be flashed ([`crate::safety::Write::FlashSuper`]).

use crate::lp::{self, ATTR_READONLY, Extent, Metadata, Partition, SECTOR, TARGET_LINEAR};
use crate::sparse::{self, BLOCK, Span};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Sectors in a 4 KiB block: extents start and end on blocks, as the sparse image writes them.
const BLOCK_SECTORS: u64 = BLOCK / SECTOR;

/// What the plan replaces; everything else in `super` is kept where it is.
fn replaced(name: &str) -> bool {
    matches!(name, "system_a" | "system_b" | "product_a" | "product_b")
}

fn other(slot: char) -> char {
    if slot == 'a' { 'b' } else { 'a' }
}

/// New metadata for `super`: the R1's own [current], with r1os's system ([system_bytes] long) for [slot] and, when its
/// vendor is there, the other slot; `product_*` gone; everything else where it was.
pub fn plan(current: &Metadata, slot: char, system_bytes: u64) -> Result<Metadata> {
    if slot != 'a' && slot != 'b' {
        bail!("no slot {slot}");
    }
    let [dev] = current.block_devices.as_slice() else { bail!("super spans more than one block device: not something this rewrites") };
    if dev.name != "super" || dev.first_logical_sector % BLOCK_SECTORS != 0 || !(current.geometry.logical_block_size as u64).is_multiple_of(BLOCK) {
        bail!("a super this doesn't know: {dev:?}");
    }
    if system_bytes == 0 || !system_bytes.is_multiple_of(BLOCK) {
        bail!("the system image isn't a whole number of 4 KiB blocks ({system_bytes} bytes)");
    }
    let vendor = |s: char| current.partition(&format!("vendor_{s}")).filter(|p| p.sectors() > 0);
    let mine = vendor(slot).with_context(|| format!("vendor_{slot} is empty: slot {slot} wouldn't boot. Rabbit's restore puts it back; don't flash this slot"))?;
    let mut slots = vec![(slot, mine.group)];
    if let Some(v) = vendor(other(slot)) {
        slots.push((other(slot), v.group));
    }
    for p in &current.partitions {
        if p.extents.iter().any(|e| e.target_type != TARGET_LINEAR || e.target_source != 0) && !replaced(&p.name) {
            bail!("{} isn't a plain linear partition: not something this keeps", p.name);
        }
    }
    let kept: Vec<Partition> = current.partitions.iter().filter(|p| !replaced(&p.name)).cloned().collect();

    // Free space: super's logical sectors, less every kept extent, on whole blocks.
    let end = (dev.size / SECTOR) / BLOCK_SECTORS * BLOCK_SECTORS;
    let mut used: Vec<(u64, u64)> = kept.iter().flat_map(|p| p.extents.iter().map(|e| (e.target_data, e.target_data + e.sectors))).collect();
    used.sort();
    let mut free = Vec::new();
    let mut at = dev.first_logical_sector;
    for (s, e) in used.iter().copied().chain(std::iter::once((end, end))) {
        let (a, b) = (at.div_ceil(BLOCK_SECTORS) * BLOCK_SECTORS, s / BLOCK_SECTORS * BLOCK_SECTORS);
        if b > a {
            free.push((a, b));
        }
        at = at.max(e);
    }

    let need = system_bytes / SECTOR;
    let mut new = Vec::new();
    for &(s, group) in &slots {
        // The first gap it fits in whole, else across gaps in order.
        let mut extents = Vec::new();
        if let Some(i) = free.iter().position(|&(a, b)| b - a >= need) {
            extents.push(Extent { sectors: need, target_type: TARGET_LINEAR, target_data: free[i].0, target_source: 0 });
            free[i].0 += need;
        } else {
            let mut left = need;
            for gap in free.iter_mut() {
                if left == 0 {
                    break;
                }
                let take = left.min(gap.1 - gap.0);
                if take > 0 {
                    extents.push(Extent { sectors: take, target_type: TARGET_LINEAR, target_data: gap.0, target_source: 0 });
                    gap.0 += take;
                    left -= take;
                }
            }
            if left > 0 {
                bail!("no room in super for system_{s} ({} MB short)", (left * SECTOR) >> 20);
            }
        }
        new.push(Partition { name: format!("system_{s}"), attributes: ATTR_READONLY, group, extents });
    }
    new.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = current.clone();
    out.partitions = new.into_iter().chain(kept).collect();
    for (i, g) in out.groups.iter().enumerate() {
        let size: u64 = out.partitions.iter().filter(|p| p.group as usize == i).map(|p| p.sectors() * SECTOR).sum();
        if g.maximum_size > 0 && size > g.maximum_size {
            bail!("group {} would hold {size} bytes, more than its {}", g.name, g.maximum_size);
        }
    }
    // It must write out and read back as itself.
    out.to_bytes()?;
    Ok(out)
}

/// Where Android keeps its Virtual A/B message in `misc` (`para` on the R1): `SYSTEM_SPACE_OFFSET_IN_MISC`.
pub const VIRTUAL_AB_OFFSET: usize = 32 * 1024;
const VIRTUAL_AB_MAGIC: u32 = 0x5674_0ab0;

/// A Rabbit update's merge status, from `para` (`misc_virtual_ab_message`, as the boot HAL keeps it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    /// No update, or never one (no message).
    None,
    Unknown,
    /// An update's applied and waiting for its first boot.
    Snapshotted,
    Merging,
    Cancelled,
}

/// The merge status in [para] (its first 64 KiB or more, from the backup or `adb`). Rewriting `super` under a pending
/// update's snapshots would break it, so only [MergeStatus::None] goes ahead (ADR-0067).
pub fn merge_status(para: &[u8]) -> Result<MergeStatus> {
    let m = para.get(VIRTUAL_AB_OFFSET..VIRTUAL_AB_OFFSET + 7).context("too little of para to hold its Virtual A/B message")?;
    if u32::from_le_bytes(m[1..5].try_into().unwrap()) != VIRTUAL_AB_MAGIC {
        return Ok(MergeStatus::None);
    }
    Ok(match m[5] {
        0 => MergeStatus::None,
        1 => MergeStatus::Unknown,
        2 => MergeStatus::Snapshotted,
        3 => MergeStatus::Merging,
        4 => MergeStatus::Cancelled,
        s => bail!("a merge status this doesn't know: {s}"),
    })
}

/// A `super` image that [verify] passed: the only kind [`crate::safety::Write::FlashSuper`] takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuperImage {
    path: PathBuf,
    /// The system image's SHA-256, as written at each of its extents.
    pub system_sha256: String,
    pub metadata: Metadata,
    /// ADR-0067's no-op probe ([probe]): the metadata region exactly as read, by its SHA-256; nothing else.
    probe: Option<String>,
}

impl SuperImage {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_probe(&self) -> bool {
        self.probe.is_some()
    }
}

/// ADR-0067's no-op probe: an image that writes `super`'s metadata region back exactly as [head] (`super`'s first
/// bytes, read from the R1) has it, and leaves everything else as it is. Flashed from the bootloader, it changes
/// nothing if the bootloader skips "don't care" chunks, and shows that it does.
pub fn probe(head: &[u8], out: &Path) -> Result<SuperImage> {
    let current = lp::read(head, 0)?;
    let dev = current.block_devices.first().context("no block device")?;
    let (a, b) = (lp::METADATA_OFFSET as usize, (lp::METADATA_OFFSET + current.geometry.region_len()) as usize);
    let region = head.get(a..b).context("too little of super to hold its metadata region")?;
    let mut w = sparse::Writer::new(std::io::BufWriter::new(std::fs::File::create(out)?), dev.size / BLOCK)?;
    w.skip_to(lp::METADATA_OFFSET / BLOCK)?;
    w.data(region)?;
    w.finish()?;
    let sum = crate::download::hex(&Sha256::digest(region));
    let image = SuperImage { path: out.to_path_buf(), system_sha256: String::new(), metadata: current.clone(), probe: Some(sum) };
    verify(&current, &image)?;
    Ok(image)
}

/// Writes [plan]'s `super` image for [current] into [out]: the metadata, then [system] at each new system extent.
/// [progress] hears the bytes of system written and the total. Verified before it's returned.
pub fn build(current: &Metadata, slot: char, system: &Path, out: &Path, mut progress: impl FnMut(u64, u64)) -> Result<SuperImage> {
    let system_bytes = std::fs::metadata(system).with_context(|| format!("no system image at {}", system.display()))?.len();
    let new = plan(current, slot, system_bytes)?;
    let dev = &current.block_devices[0];
    if !dev.size.is_multiple_of(BLOCK) {
        bail!("super isn't a whole number of 4 KiB blocks");
    }
    // What's written where: the metadata region, then every new system extent in order on the device.
    let mut writes: Vec<(u64, u64, Option<u64>)> = vec![(lp::METADATA_OFFSET, current.geometry.region_len(), None)];
    for p in new.partitions.iter().filter(|p| p.name.starts_with("system_")) {
        let mut logical = 0u64;
        for e in &p.extents {
            writes.push((e.target_data * SECTOR, e.sectors * SECTOR, Some(logical)));
            logical += e.sectors * SECTOR;
        }
    }
    writes.sort();
    let region = new.region()?;
    let total = system_bytes * new.partitions.iter().filter(|p| p.name.starts_with("system_")).count() as u64;
    let mut done = 0u64;
    let mut img = std::fs::File::open(system)?;
    let file = std::fs::File::create(out)?;
    let mut w = sparse::Writer::new(std::io::BufWriter::with_capacity(8 << 20, file), dev.size / BLOCK)?;
    let mut buf = vec![0u8; 4 << 20];
    for (at, len, from) in writes {
        if at % BLOCK != 0 || len % BLOCK != 0 {
            bail!("a write not on 4 KiB blocks ({at}, {len})");
        }
        w.skip_to(at / BLOCK)?;
        match from {
            None => w.data(&region)?,
            Some(logical) => {
                img.seek(SeekFrom::Start(logical))?;
                let mut left = len;
                while left > 0 {
                    let n = left.min(buf.len() as u64) as usize;
                    img.read_exact(&mut buf[..n])?;
                    w.data(&buf[..n])?;
                    left -= n as u64;
                    done += n as u64;
                    progress(done, total);
                }
            }
        }
    }
    w.finish()?;
    let system_sha256 = crate::download::sha256_of(system)?;
    let image = SuperImage { path: out.to_path_buf(), system_sha256, metadata: new, probe: None };
    verify(current, &image)?;
    Ok(image)
}

/// Reads [image] back: every byte it writes is in the metadata region or a new system extent; none touches a partition
/// [current] has and the plan keeps; the metadata it writes is the plan's, in every slot; and each system copy is the
/// system image, by SHA-256.
pub fn verify(current: &Metadata, image: &SuperImage) -> Result<()> {
    let (size, spans) = sparse::spans(&image.path)?;
    let dev = &current.block_devices[0];
    if size != dev.size {
        bail!("the image is {size} bytes, super is {}", dev.size);
    }
    let region = (lp::METADATA_OFFSET, lp::METADATA_OFFSET + current.geometry.region_len());
    let systems: Vec<&Partition> =
        if image.probe.is_some() { Vec::new() } else { image.metadata.partitions.iter().filter(|p| p.name.starts_with("system_")).collect() };
    let mut allowed = vec![region];
    for p in &systems {
        allowed.extend(p.extents.iter().map(|e| (e.target_data * SECTOR, (e.target_data + e.sectors) * SECTOR)));
    }
    let kept: Vec<(String, u64, u64)> = current
        .partitions
        .iter()
        .filter(|p| !replaced(&p.name))
        .flat_map(|p| p.extents.iter().map(move |e| (p.name.clone(), e.target_data * SECTOR, (e.target_data + e.sectors) * SECTOR)))
        .collect();
    for s in &spans {
        let (a, b) = s.range();
        if let Some((name, _, _)) = kept.iter().find(|(_, ka, kb)| a < *kb && *ka < b) {
            bail!("the image writes over {name} (bytes {a}..{b}): refused");
        }
        if !allowed.iter().any(|&(x, y)| x <= a && b <= y) {
            bail!("the image writes bytes {a}..{b}, outside the metadata and the new system: refused");
        }
    }
    // The device as the image leaves it, read where it writes.
    let mut f = std::fs::File::open(&image.path)?;
    let mut read = |at: u64, len: u64| -> Result<Vec<u8>> {
        let mut out = vec![0u8; len as usize];
        let mut covered = 0u64;
        for s in &spans {
            let (a, b) = s.range();
            let (x, y) = (a.max(at), b.min(at + len));
            if x >= y {
                continue;
            }
            match *s {
                Span::Raw { file_at, .. } => {
                    f.seek(SeekFrom::Start(file_at + (x - a)))?;
                    f.read_exact(&mut out[(x - at) as usize..(y - at) as usize])?;
                }
                Span::Fill { value, .. } => {
                    for (k, b) in out[(x - at) as usize..(y - at) as usize].iter_mut().enumerate() {
                        *b = value.to_le_bytes()[(x - a + k as u64) as usize % 4];
                    }
                }
            }
            covered += y - x;
        }
        if covered != len {
            bail!("the image doesn't write all of bytes {at}..{} ({covered} of {len})", at + len);
        }
        Ok(out)
    };
    let mut head = vec![0u8; region.1 as usize];
    head[region.0 as usize..].copy_from_slice(&read(region.0, region.1 - region.0)?);
    if let Some(sum) = &image.probe {
        // The probe writes the region exactly as it was read, and nothing else.
        if crate::download::hex(&Sha256::digest(&head[region.0 as usize..])) != *sum {
            bail!("the probe doesn't write the metadata region as it was read");
        }
        return Ok(());
    }
    // The geometry isn't written: the R1's own stays. Read the slots back against it.
    let geometry = current.geometry.clone();
    for slot in 0..geometry.metadata_slot_count {
        let max = geometry.metadata_max_size as u64;
        let mut one = head[(lp::METADATA_OFFSET + slot as u64 * max) as usize..][..max as usize].to_vec();
        let backup = head[(lp::METADATA_OFFSET + (geometry.metadata_slot_count + slot) as u64 * max) as usize..][..max as usize].to_vec();
        if one != backup {
            bail!("metadata slot {slot} and its backup differ in the image");
        }
        one.resize(max as usize, 0);
        let bytes = image.metadata.to_bytes()?;
        if one[..bytes.len()] != bytes[..] {
            bail!("metadata slot {slot} in the image isn't the plan's");
        }
    }
    for p in systems {
        let mut h = Sha256::new();
        for e in &p.extents {
            let mut at = e.target_data * SECTOR;
            let end = at + e.sectors * SECTOR;
            while at < end {
                let n = (end - at).min(8 << 20);
                h.update(read(at, n)?);
                at += n;
            }
        }
        if crate::download::hex(&h.finalize()) != image.system_sha256 {
            bail!("{} in the image isn't the system image", p.name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lp::{BlockDevice, Group};

    fn lin(sectors: u64, at: u64) -> Extent {
        Extent { sectors, target_type: TARGET_LINEAR, target_data: at, target_source: 0 }
    }

    /// A small super for building whole images in tests: 16 MiB, vendor on both slots, an old system and product.
    fn small(vendor_b: u64) -> Metadata {
        Metadata {
            geometry: lp::Geometry { metadata_max_size: 4096, metadata_slot_count: 3, logical_block_size: 4096 },
            major: 10,
            minor: 2,
            header_size: 256,
            flags: lp::FLAG_VIRTUAL_AB,
            partitions: vec![
                Partition { name: "system_a".into(), attributes: ATTR_READONLY, group: 1, extents: vec![lin(4096, 2048)] },
                Partition { name: "product_a".into(), attributes: ATTR_READONLY, group: 1, extents: vec![lin(2048, 6144)] },
                Partition { name: "vendor_a".into(), attributes: ATTR_READONLY, group: 1, extents: vec![lin(2048, 8192)] },
                Partition {
                    name: "vendor_b".into(),
                    attributes: ATTR_READONLY,
                    group: 2,
                    extents: if vendor_b > 0 { vec![lin(vendor_b, 20480)] } else { vec![] },
                },
            ],
            groups: vec![
                Group { name: "default".into(), flags: 0, maximum_size: 0 },
                Group { name: "main_a".into(), flags: 0, maximum_size: 15 << 20 },
                Group { name: "main_b".into(), flags: 0, maximum_size: 15 << 20 },
            ],
            block_devices: vec![BlockDevice { first_logical_sector: 2048, alignment: 512, alignment_offset: 0, size: 16 << 20, name: "super".into(), flags: 0 }],
        }
    }

    fn overlaps(a: &Partition, b: &Partition) -> bool {
        a.extents.iter().any(|x| b.extents.iter().any(|y| x.target_data < y.target_data + y.sectors && y.target_data < x.target_data + x.sectors))
    }

    #[test]
    fn the_r1_s_super_planned() {
        let (_, current) = crate::lp::tests::r1_like();
        let system = 1_972_994_048u64; // r1os 0.2.18
        let new = plan(&current, 'a', system).unwrap();
        let names: Vec<&str> = new.partitions.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["system_a", "system_b", "vendor_a", "vendor_b"]);
        // The vendor partitions, exactly as they were.
        for v in ["vendor_a", "vendor_b"] {
            assert_eq!(new.partition(v), current.partition(v), "{v}");
        }
        for s in ["system_a", "system_b"] {
            let p = new.partition(s).unwrap();
            assert_eq!(p.sectors() * SECTOR, system);
            assert!(p.extents.iter().all(|e| e.target_data % BLOCK_SECTORS == 0 && e.target_data >= 2048));
            for v in ["vendor_a", "vendor_b"] {
                assert!(!overlaps(p, new.partition(v).unwrap()), "{s} over {v}");
            }
        }
        assert!(!overlaps(new.partition("system_a").unwrap(), new.partition("system_b").unwrap()));
        assert_eq!((new.partition("system_a").unwrap().group, new.partition("system_b").unwrap().group), (1, 2));
        assert_eq!((new.flags, new.header_size, &new.geometry), (current.flags, current.header_size, &current.geometry));
    }

    #[test]
    fn what_it_refuses() {
        let (_, current) = crate::lp::tests::r1_like();
        assert!(plan(&current, 'a', 4097).is_err(), "not whole blocks");
        assert!(plan(&current, 'c', 4096).is_err());
        assert!(plan(&current, 'a', 9 << 30).is_err(), "no room");
        let mut no_vendor = current.clone();
        no_vendor.partitions.retain(|p| p.name != "vendor_a");
        assert!(format!("{:#}", plan(&no_vendor, 'a', 4096).unwrap_err()).contains("vendor_a is empty"));
        let mut two = current.clone();
        two.block_devices.push(two.block_devices[0].clone());
        assert!(plan(&two, 'a', 4096).is_err());
    }

    #[test]
    fn no_other_vendor_no_other_system() {
        let new = plan(&small(0), 'a', 1 << 20).unwrap();
        let names: Vec<&str> = new.partitions.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["system_a", "vendor_a", "vendor_b"], "product_a gone, no system_b without vendor_b");
    }

    #[test]
    fn built_writes_only_the_metadata_and_the_system() {
        let dir = std::env::temp_dir().join(format!("r1-usb-super-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let system = dir.join("system.img");
        let mut data = vec![0u8; 2 << 20];
        for (i, b) in data.iter_mut().enumerate().take(1 << 20) {
            *b = (i % 251) as u8;
        }
        std::fs::write(&system, &data).unwrap();
        let current = small(2048);
        let image = build(&current, 'a', &system, &dir.join("super.img"), |_, _| {}).unwrap();
        let (_, spans) = sparse::spans(image.path()).unwrap();
        // Vendor's sectors never written.
        for v in ["vendor_a", "vendor_b"] {
            let e = &current.partition(v).unwrap().extents[0];
            let (va, vb) = (e.target_data * SECTOR, (e.target_data + e.sectors) * SECTOR);
            assert!(spans.iter().all(|s| s.range().1 <= va || vb <= s.range().0), "{v} written");
        }
        assert_eq!(image.metadata.partition("system_b").unwrap().sectors() * SECTOR, 2 << 20);

        // An image that writes over vendor is refused.
        let mut w = sparse::Writer::new(std::fs::File::create(dir.join("bad.img")).unwrap(), (16 << 20) / BLOCK).unwrap();
        w.skip_to(8192 * SECTOR / BLOCK).unwrap();
        w.data(&[1u8; BLOCK as usize]).unwrap();
        w.finish().unwrap();
        let bad = SuperImage { path: dir.join("bad.img"), ..image.clone() };
        assert!(format!("{:#}", verify(&current, &bad).unwrap_err()).contains("writes over vendor_a"));
        // And one whose system isn't the image.
        let wrong = SuperImage { system_sha256: "00".repeat(32), ..image };
        assert!(verify(&current, &wrong).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_status_from_para() {
        let mut para = vec![0u8; 64 * 1024];
        assert_eq!(merge_status(&para).unwrap(), MergeStatus::None, "no message: never an update");
        // Max's R1, 2026-10-14: version 2, the magic, NONE, source slot 1.
        para[VIRTUAL_AB_OFFSET..VIRTUAL_AB_OFFSET + 7].copy_from_slice(&[0x02, 0xb0, 0x0a, 0x74, 0x56, 0x00, 0x01]);
        assert_eq!(merge_status(&para).unwrap(), MergeStatus::None);
        para[VIRTUAL_AB_OFFSET + 5] = 3;
        assert_eq!(merge_status(&para).unwrap(), MergeStatus::Merging);
        para[VIRTUAL_AB_OFFSET + 5] = 9;
        assert!(merge_status(&para).is_err());
        assert!(merge_status(&para[..1000]).is_err());
    }

    #[test]
    fn the_probe_rewrites_only_what_s_there() {
        let (head, current) = crate::lp::tests::r1_like();
        let dir = std::env::temp_dir().join(format!("r1-usb-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = probe(&head, &dir.join("probe.img")).unwrap();
        assert!(p.is_probe());
        let (size, spans) = sparse::spans(p.path()).unwrap();
        assert_eq!(size, 8_792_064_000);
        let (a, b) = (lp::METADATA_OFFSET, lp::METADATA_OFFSET + current.geometry.region_len());
        assert!(spans.iter().all(|s| a <= s.range().0 && s.range().1 <= b), "only the metadata region");
        // A probe claiming other bytes than the R1's is refused.
        let lie = SuperImage { probe: Some("00".repeat(32)), ..p };
        assert!(verify(&current, &lie).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
