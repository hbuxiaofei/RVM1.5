use libvmm::msr::Msr;
use x86::{segmentation, segmentation::SegmentSelector, task};
use x86_64::registers::control::{Cr0, Cr0Flags, Cr3, Cr3Flags, Cr4, Cr4Flags};
use x86_64::{addr::PhysAddr, structures::paging::PhysFrame, structures::DescriptorTablePointer};

use super::segmentation::Segment;
use super::tables::{GDTStruct, IDTStruct, GDT, IDT};

const SAVED_LINUX_REGS: usize = 7;

#[derive(Debug)]
pub struct LinuxContext {
    pub rsp: usize,
    pub rip: usize,

    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub rbx: usize,
    pub rbp: usize,

    pub cs: Segment,
    pub ds: Segment,
    pub es: Segment,
    pub fs: Segment,
    pub gs: Segment,
    pub tss: Segment,
    pub gdt: DescriptorTablePointer,
    pub idt: DescriptorTablePointer,

    pub cr0: Cr0Flags,
    pub cr3: usize,
    pub cr4: Cr4Flags,

    pub efer: u64,
    pub lstar: u64,
    pub pat: u64,
    pub mtrr_def_type: u64,
}

#[repr(C)]
#[derive(Debug)]
pub struct GuestRegisters {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rdi: usize,
    pub rsi: usize,
    pub rbp: usize,
    _unsed_rsp: usize,
    pub rbx: usize,
    pub rdx: usize,
    pub rcx: usize,
    pub rax: usize,
}

impl LinuxContext {
    pub fn load_from(linux_sp: usize) -> Self {
        let regs =
            unsafe { core::slice::from_raw_parts(linux_sp as *const usize, SAVED_LINUX_REGS) };

        let gdt = GDTStruct::sgdt();

        let mut fs = Segment::from_selector(segmentation::fs(), &gdt);
        let mut gs = Segment::from_selector(segmentation::gs(), &gdt);
        fs.base = Msr::IA32_FS_BASE.read();
        gs.base = Msr::IA32_GS_BASE.read();

        let ret = Self {
            rsp: regs.as_ptr_range().end as usize,
            r15: regs[0],
            r14: regs[1],
            r13: regs[2],
            r12: regs[3],
            rbx: regs[4],
            rbp: regs[5],
            rip: regs[6],
            cs: Segment::from_selector(segmentation::cs(), &gdt),
            ds: Segment::from_selector(segmentation::ds(), &gdt),
            es: Segment::from_selector(segmentation::es(), &gdt),
            fs,
            gs,
            tss: Segment::from_selector(task::tr(), &gdt),
            gdt,
            idt: IDTStruct::sidt(),
            cr0: Cr0::read(),
            cr3: Cr3::read().0.start_address().as_u64() as usize,
            cr4: Cr4::read(),
            efer: Msr::IA32_EFER.read(),
            lstar: Msr::IA32_LSTAR.read(),
            pat: Msr::IA32_PAT.read(),
            mtrr_def_type: Msr::IA32_MTRR_DEF_TYPE.read(),
        };

        // Setup new GDT, IDT, CS, TSS
        GDT.lock().load();
        unsafe {
            segmentation::load_cs(GDTStruct::KCODE_SELECTOR);
            segmentation::load_ds(SegmentSelector::from_raw(0));
            segmentation::load_es(SegmentSelector::from_raw(0));
            segmentation::load_ss(SegmentSelector::from_raw(0));
        }
        IDT.lock().load();
        GDT.lock().load_tss(GDTStruct::TSS_SELECTOR);

        // PAT0: WB, PAT1: WC, PAT2: UC
        unsafe { Msr::IA32_PAT.write(0x070106) };

        ret
    }

    pub fn restore(&self) {
        unsafe {
            Msr::IA32_PAT.write(self.pat);
            Msr::IA32_EFER.write(self.efer);

            Cr0::write(self.cr0);
            Cr4::write(self.cr4);
            // cr3 must be last in case cr4 enables PCID
            Cr3::write(
                PhysFrame::containing_address(PhysAddr::new(self.cr3 as _)),
                Cr3Flags::empty(), // clear PCID
            );

            // Copy Linux TSS descriptor into our GDT, clearing the busy flag,
            // then reload TR from it. We can't use Linux' GDT as it is r/o.
            {
                let mut hv_gdt_lock = GDT.lock();
                let hv_gdt = GDTStruct::table_of_mut(hv_gdt_lock.pointer());
                let liunx_gdt = GDTStruct::table_of(&self.gdt);
                let tss_idx = self.tss.selector.index() as usize;
                hv_gdt[tss_idx] = liunx_gdt[tss_idx];
                hv_gdt[tss_idx + 1] = liunx_gdt[tss_idx + 1];
                hv_gdt_lock.load_tss(self.tss.selector);
            }

            GDTStruct::lgdt(&self.gdt);
            IDTStruct::lidt(&self.idt);

            segmentation::load_cs(self.cs.selector); // XXX: failed to swtich to user CS
            segmentation::load_ds(self.ds.selector);
            segmentation::load_es(self.es.selector);
            segmentation::load_fs(self.fs.selector);
            segmentation::load_gs(self.gs.selector);

            Msr::IA32_FS_BASE.write(self.fs.base);
            Msr::IA32_GS_BASE.write(self.gs.base);
        }
    }
}

impl GuestRegisters {
    pub fn set_return(&mut self, ret: usize) {
        self.rax = ret
    }
}