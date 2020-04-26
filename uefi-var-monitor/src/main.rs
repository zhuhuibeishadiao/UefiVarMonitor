#![no_main]
#![no_std]

use r_efi::efi;

#[macro_use]
mod serial;

type GetVariableType = extern "win64" fn(
    *mut r_efi::base::Char16,
    *mut r_efi::base::Guid,
    *mut u32,
    *mut usize,
    *mut core::ffi::c_void,
) -> r_efi::base::Status;

static mut GET_VARIABLE: GetVariableType = handle_get_variable;
static mut RUNTIME: *mut r_efi::efi::RuntimeServices = core::ptr::null_mut();

/**
 * @brief Handles GetVariable runtime service calls.
 */
extern "win64" fn handle_get_variable(
    variable_name: *mut r_efi::base::Char16,
    vendor_guid: *mut r_efi::base::Guid,
    attributes: *mut u32,
    data_size: *mut usize,
    data: *mut core::ffi::c_void,
) -> efi::Status {
    //
    // Invoke the original GetVariable service, and log this service invocation.
    //
    let efi_status =
        unsafe { GET_VARIABLE(variable_name, vendor_guid, attributes, data_size, data) };

    //
    // Convert to UTF-8 form USC-2 up to 64 characters.
    //
    let mut var_name_buffer = [0; 64];
    let mut i: usize = 0;
    while i < var_name_buffer.len() {
        unsafe {
            var_name_buffer[i] =
                (*(((variable_name as u64) + (2 * i as u64)) as *const u16) & 0xffu16) as u8;
        }
        if var_name_buffer[i] == 0 {
            break;
        }
        i += 1;
    }
    let name = unsafe { core::str::from_utf8_unchecked(&var_name_buffer) };
    let effective_size = if data_size.is_null() {
        0
    } else {
        unsafe { *data_size }
    };
    let data = unsafe { (*vendor_guid).as_fields() };
    log!(
        "G: {:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X} Size={:08x} {}: {:#x}",
        data.0,
        data.1,
        data.2,
        data.3,
        data.4,
        data.5[0],
        data.5[1],
        data.5[2],
        data.5[3],
        data.5[4],
        data.5[5],
        effective_size,
        name,
        efi_status.as_usize(),
    );

    return efi_status;
}

/**
 * @brief Converts global pointers from physical-mode ones to virtual-mode ones.
 */
extern "win64" fn handle_set_virtual_address_map(
    _event: r_efi::base::Event,
    _context: *mut core::ffi::c_void,
) {
    let current = unsafe { GET_VARIABLE };
    let efi_status = unsafe {
        ((*RUNTIME).convert_pointer)(
            0,
            &mut GET_VARIABLE as *mut _ as *mut *mut core::ffi::c_void,
        )
    };
    unsafe {
        log!(
            "GetVariable relocated from {:#08x} to {:#08x}",
            current as u64,
            GET_VARIABLE as u64,
        );
    }
    assert!(!efi_status.is_error());
}

/**
 * @brief Exchanges a pointer in the EFI System Table.
 */
fn exchange_pointer_in_service_table(
    system_table: *mut efi::SystemTable,
    address_to_update: *mut *mut core::ffi::c_void,
    new_function_pointer: *mut core::ffi::c_void,
    original_function_pointer: *mut *mut core::ffi::c_void,
) -> efi::Status {
    unsafe { assert!(*address_to_update != new_function_pointer) };

    //
    // Disable interrupt.
    //
    let tpl = unsafe { ((*(*system_table).boot_services).raise_tpl)(efi::TPL_HIGH_LEVEL) };

    unsafe {
        *original_function_pointer = *address_to_update;
        *address_to_update = new_function_pointer;
    };

    //
    // Update the CRC32 in the EFI System Table header.
    //
    unsafe { (*system_table).hdr.crc32 = 0 };
    let efi_status = unsafe {
        ((*(*system_table).boot_services).calculate_crc32)(
            &mut (*system_table).hdr as *mut _ as *mut core::ffi::c_void,
            (*system_table).hdr.header_size as usize,
            &mut (*system_table).hdr.crc32,
        )
    };
    assert!(!efi_status.is_error());

    unsafe { ((*(*system_table).boot_services).restore_tpl)(tpl) };
    return efi_status;
}

/**
 * @brief The module entry point.
 */
#[no_mangle]
fn efi_main(_image_handle: efi::Handle, system_table: *mut efi::SystemTable) -> efi::Status {
    log!("Driver being loaded");

    unsafe {
        RUNTIME = (*system_table).runtime_services;
    };
    let boot_services = unsafe { (*system_table).boot_services };

    //
    // Register a notification for SetVirtualAddressMap call.
    //
    let mut event: r_efi::base::Event = core::ptr::null_mut();
    let mut efi_status = unsafe {
        ((*boot_services).create_event_ex)(
            r_efi::efi::EVT_NOTIFY_SIGNAL,
            r_efi::efi::TPL_CALLBACK,
            handle_set_virtual_address_map,
            core::ptr::null_mut(),
            &mut r_efi::efi::EVENT_GROUP_VIRTUAL_ADDRESS_CHANGE,
            &mut event,
        )
    };
    if efi_status.is_error() {
        log!("create_event_ex failed : {:#x}", efi_status.as_usize());
        return efi_status;
    }

    //
    // Install hooks.
    //
    efi_status = unsafe {
        exchange_pointer_in_service_table(
            system_table,
            &mut (*(*system_table).runtime_services).get_variable as *mut _
                as *mut *mut core::ffi::c_void,
            handle_get_variable as *mut core::ffi::c_void,
            &mut GET_VARIABLE as *mut _ as *mut *mut core::ffi::c_void,
        )
    };
    if efi_status.is_error() {
        log!(
            "exchange_table_pointer failed : {:#x}",
            efi_status.as_usize()
        );
        unsafe { ((*boot_services).close_event)(event) };
        return efi_status;
    }

    return efi_status;
}

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}