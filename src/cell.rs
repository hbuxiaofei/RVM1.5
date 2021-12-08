use crate::arch::{HostPageTable, NestedPageTable};
use crate::config::{CellConfig, HvSystemConfig};
use crate::consts::HV_BASE;
use crate::error::HvResult;
use crate::header::HvHeader;
use crate::memory::addr::{phys_to_virt, GuestPhysAddr, HostPhysAddr};
use crate::memory::{MemFlags, MemoryRegion, MemorySet};

#[derive(Debug)]
pub struct Cell<'a> {
    /// Cell configuration.
    pub config: CellConfig<'a>,
    /// Guest physical memory set.
    pub gpm: MemorySet<NestedPageTable>,
    /// Host virtual memory set.
    pub hvm: MemorySet<HostPageTable>,
}

impl Cell<'_> {
    fn new_root() -> HvResult<Self> {
        let header = HvHeader::get();
        let sys_config = HvSystemConfig::get();
        let cell_config = sys_config.root_cell.config();

        let hv_phys_start = sys_config.hypervisor_memory.phys_start as usize;
        let hv_phys_size = sys_config.hypervisor_memory.size as usize;
        let mut gpm = MemorySet::new();
        let mut hvm = MemorySet::new();

        // Init guest physical memory set, create hypervisor page table.
        //
        // hypervisor
        gpm.insert(MemoryRegion::new_with_empty_mapper(
            hv_phys_start,
            hv_phys_size,
            MemFlags::READ | MemFlags::NO_HUGEPAGES,
        ))?;
        // all physical memory regions
        for region in cell_config.mem_regions() {
            gpm.insert(MemoryRegion::new_with_offset_mapper(
                region.virt_start as GuestPhysAddr,
                region.phys_start as HostPhysAddr,
                region.size as usize,
                region.flags,
            ))?;
        }

        // hypervisor core
        // TODO: Fine-grained permissions setting
        hvm.insert(MemoryRegion::new_with_offset_mapper(
            HV_BASE,
            hv_phys_start,
            header.core_size,
            MemFlags::READ | MemFlags::WRITE | MemFlags::EXECUTE,
        ))?;
        // per-CPU data, configurations & page pool
        hvm.insert(MemoryRegion::new_with_offset_mapper(
            HV_BASE + header.core_size,
            hv_phys_start + header.core_size,
            hv_phys_size - header.core_size,
            MemFlags::READ | MemFlags::WRITE,
        ))?;
        // to directly access all guest RAM
        for region in cell_config.mem_regions() {
            if region.flags.contains(MemFlags::EXECUTE) {
                let hv_virt_start = phys_to_virt(region.virt_start as GuestPhysAddr);
                if hv_virt_start < region.virt_start as GuestPhysAddr {
                    return hv_result_err!(
                        EINVAL,
                        format!(
                            "Guest physical address {:#x} is too large",
                            region.virt_start
                        )
                    );
                }
                hvm.insert(MemoryRegion::new_with_offset_mapper(
                    hv_virt_start,
                    region.phys_start as HostPhysAddr,
                    region.size as usize,
                    region.flags,
                ))?;
            }
        }
        trace!("Guest phyiscal memory set: {:#x?}", gpm);
        trace!("Hypervisor virtual memory set: {:#x?}", hvm);

        Ok(Self {
            config: cell_config,
            gpm,
            hvm,
        })
    }
}

lazy_static! {
    pub static ref ROOT_CELL: Cell<'static> = Cell::new_root().unwrap();
}

pub fn init() -> HvResult {
    crate::arch::vmm::check_hypervisor_feature()?;

    lazy_static::initialize(&ROOT_CELL);

    info!("Root cell init end.");
    debug!("{:#x?}", &*ROOT_CELL);
    Ok(())
}
