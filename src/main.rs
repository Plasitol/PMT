use crate::egui::Stroke;
use crate::egui::Rounding;
use crate::egui::Pos2;
use crate::egui::Rect;
use crate::egui::Ui;
use core::mem::size_of;
use winapi::um::winioctl::IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use widestring::U16CString;
use winapi::um::fileapi::{CreateFileW, GetDiskFreeSpaceExW, GetLogicalDriveStringsW};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::winnt::{GENERIC_READ, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_ATTRIBUTE_NORMAL};
use winapi::um::ioapiset::DeviceIoControl;
use winapi::um::winioctl::{IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, DISK_GEOMETRY_EX};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winioctl::{IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_QUERY, StorageDeviceProperty};
use winapi::um::winnt::ULARGE_INTEGER;
use eframe::{egui, NativeOptions};
use winapi::um::winioctl::{DRIVE_LAYOUT_INFORMATION_EX, PARTITION_INFORMATION_EX, IOCTL_DISK_GET_DRIVE_LAYOUT_EX};
use egui::Color32;
use crate::egui::Vec2;

#[repr(C)]
struct STORAGE_DEVICE_DESCRIPTOR {
    Version: u32,
    Size: u32,
    DeviceType: u8,
    DeviceTypeModifier: u8,
    RemovableMedia: u8,
    CommandQueueing: u8,
    VendorIdOffset: u32,
    ProductIdOffset: u32,
    ProductRevisionOffset: u32,
    SerialNumberOffset: u32,
    BusType: u8,
    RawPropertiesLength: u32,
    RawDeviceProperties: [u8; 1],
}

#[repr(C)]
struct VOLUME_DISK_EXTENTS {
    NumberOfDiskExtents: u32,
    Extents: [DISK_EXTENT; 1],
}

#[repr(C)]
struct DISK_EXTENT {
    DiskNumber: u32,
    StartingOffset: i64,
    ExtentLength: u64,
}

fn get_drive_geometry(handle: winapi::um::winnt::HANDLE) -> Option<DISK_GEOMETRY_EX> {
    let mut disk_geometry_ex: DISK_GEOMETRY_EX = unsafe { std::mem::zeroed() };
    let mut bytes_returned: u32 = 0;
    let result = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
            null_mut(),
            0,
            &mut disk_geometry_ex as *mut _ as *mut _,
            std::mem::size_of::<DISK_GEOMETRY_EX>() as u32,
            &mut bytes_returned,
            null_mut(),
        )
    };
    if result == 0 {
        None
    } else {
        Some(disk_geometry_ex)
    }
}

fn get_drive_model_and_type(index: usize) -> Option<(String, String)> {
    let device_path = format!("\\\\.\\PHYSICALDRIVE{}", index);
    let device_path_utf16 = U16CString::from_str(&device_path).ok()?;
    let handle = unsafe {
        CreateFileW(
            device_path_utf16.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null_mut(),
            winapi::um::fileapi::OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: 0,
        AdditionalParameters: [0; 1],
    };
    let mut buffer = vec![0u8; 1024];
    let mut bytes_returned: u32 = 0;
    let result = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &mut query as *mut _ as *mut _,
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buffer.as_mut_ptr() as *mut _,
            buffer.len() as u32,
            &mut bytes_returned,
            null_mut(),
        )
    };
    if result == 0 {
        unsafe { CloseHandle(handle) };
        return None;
    }
    let descriptor = unsafe { &*(buffer.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    let vendor_id = if descriptor.VendorIdOffset != 0 {
        unsafe {
            let ptr = buffer.as_ptr().add(descriptor.VendorIdOffset as usize);
            Some(std::ffi::CStr::from_ptr(ptr as *const i8).to_string_lossy().into_owned())
        }
    } else {
        None
    };
    let product_id = if descriptor.ProductIdOffset != 0 {
        unsafe {
            let ptr = buffer.as_ptr().add(descriptor.ProductIdOffset as usize);
            Some(std::ffi::CStr::from_ptr(ptr as *const i8).to_string_lossy().into_owned())
        }
    } else {
        None
    };
    let bus_type = match descriptor.BusType {
        0x01 => "SCSI",
        0x02 => "ATAPI",
        0x03 => "ATA",
        0x04 => "1394",
        0x05 => "SSA",
        0x06 => "Fibre",
        0x07 => "USB",
        0x08 => "RAID",
        0x09 => "iSCSI",
        0x0A => "SAS",
        0x0B => "SATA",
        0x0C => "SD",
        0x0D => "MMC",
        0x0E => "VIRTUAL",
        0x0F => "FileBackedVirtual",
        0x10 => "Spaces",
        0x11 => "NVMe",
        0x12 => "SCM",
        0x7F => "BusTypeMaxReserved",
        _ => "UNKNOWN",
    };
    unsafe { CloseHandle(handle) };
    Some((format!("{} {}", vendor_id.unwrap_or_default(), product_id.unwrap_or_default()), bus_type.to_string()))
}

fn get_logical_drives() -> Vec<String> {
    let mut drives = Vec::new();
    let mut buffer = vec![0u16; 256];
    let len = unsafe {
        GetLogicalDriveStringsW(
            buffer.len() as u32,
            buffer.as_mut_ptr(),
        )
    };
    if len == 0 {
        return drives;
    }
    let mut start = 0;
    while start < len as usize {
        let end = buffer[start..].iter().position(|&c| c == 0).unwrap_or(buffer.len());
        if end > start {
            let drive = unsafe { U16CString::from_vec_unchecked(buffer[start..=end].to_vec()) };
            drives.push(drive.to_string_lossy().to_string());
            start = end + 1;
        } else {
            start += 1;
        }
    }
    drives
}

fn get_logical_drives_on_physical_drive(physical_drive_index: usize) -> Vec<String> {
    let mut logical_drives = Vec::new();
    let mut drives_mask = unsafe { winapi::um::fileapi::GetLogicalDrives() };
    if drives_mask == 0 {
        let error_code = unsafe { GetLastError() };

        return logical_drives;
    }

    for drive_letter in 'A'..='Z' {
        if drives_mask & 1 == 1 {
            let drive_path = format!("\\\\.\\{}:", drive_letter);
            let drive_path_utf16: Vec<u16> = drive_path.encode_utf16().chain(Some(0)).collect();
            let handle = unsafe {
                CreateFileW(
                    drive_path_utf16.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    null_mut(),
                    winapi::um::fileapi::OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                let mut extents = vec![0u8; size_of::<VOLUME_DISK_EXTENTS>() + size_of::<DISK_EXTENT>() * 26];
                let mut bytes_returned: u32 = 0;
                let result = unsafe {
                    DeviceIoControl(
                        handle,
                        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
                        null_mut(),
                        0,
                        extents.as_mut_ptr() as *mut _,
                        extents.len() as u32,
                        &mut bytes_returned,
                        null_mut(),
                    )
                };
                if result != 0 {
                    let disk_extents = unsafe { &*(extents.as_ptr() as *const VOLUME_DISK_EXTENTS) };
                    for extent in 0..disk_extents.NumberOfDiskExtents {
                        let disk_number = unsafe { disk_extents.Extents[extent as usize].DiskNumber };
                        if disk_number as usize == physical_drive_index {
                            logical_drives.push(drive_path.clone());
                            break;
                        }
                    }
                }
                unsafe { CloseHandle(handle) };
            }
        }
        drives_mask >>= 1;
    }
    logical_drives
}

struct HDDApp {
    drives: Arc<Mutex<Vec<(String, String)>>>,
    selected_drive: Option<usize>,
    geometry: Option<DISK_GEOMETRY_EX>,
    logical_drives_on_physical: Vec<String>,
    selected_logical_drive: Option<String>,
    drive_space_info: Option<(f64, f64, f64)>,
    partitions_info: Vec<(u64, Color32, String, String)>,
}

impl Default for HDDApp {
    fn default() -> Self {
        let mut drives = Vec::new();
        for i in 0.. {
            if let Some((model, bus_type)) = get_drive_model_and_type(i) {
                drives.push((model, bus_type));
            } else {
                break;
            }
        }
        Self {
            drives: Arc::new(Mutex::new(drives)),
            selected_drive: None,
            geometry: None,
            logical_drives_on_physical: Vec::new(),
            selected_logical_drive: None,
            drive_space_info: None,
            partitions_info: Vec::new(),
        }
    }
}


impl eframe::App for HDDApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Available physical drives:");
            let drives = self.drives.lock().unwrap();
            for (index, (drive, bus_type)) in drives.iter().enumerate() {
                if ui.button(format!("{}: {} [{}]", index, drive, bus_type)).clicked() {
                    self.selected_drive = Some(index);
                    let device_path = format!("\\\\.\\PHYSICALDRIVE{}", index);
                    let device_path_utf16: Vec<u16> = device_path.encode_utf16().chain(Some(0)).collect();
                    let handle = unsafe {
                        CreateFileW(
                            device_path_utf16.as_ptr(),
                            GENERIC_READ,
                            FILE_SHARE_READ | FILE_SHARE_WRITE,
                            null_mut(),
                            winapi::um::fileapi::OPEN_EXISTING,
                            FILE_ATTRIBUTE_NORMAL,
                            null_mut(),
                        )
                    };
                    if handle != INVALID_HANDLE_VALUE {
                        self.geometry = get_drive_geometry(handle);
                        unsafe { CloseHandle(handle) };
                        self.partitions_info = get_partitions_on_physical_drive(index);
                        self.logical_drives_on_physical = get_logical_drives_on_physical_drive(index);
                    }
                }
            }

            if let Some(index) = self.selected_drive {
                ui.separator();
                ui.heading(format!("Drive {} information:", index));
                if let Some(disk_geometry) = &self.geometry {
                    ui.label(format!("Cylinders: {}", unsafe { disk_geometry.Geometry.Cylinders.QuadPart() }));
                    ui.label(format!("Tracks per cylinder: {}", disk_geometry.Geometry.TracksPerCylinder));
                    ui.label(format!("Sectors per track: {}", disk_geometry.Geometry.SectorsPerTrack));
                    ui.label(format!("Bytes per sector: {}", disk_geometry.Geometry.BytesPerSector));
                } else {
                    ui.label("Failed to get disk geometry.");
                }

                ui.label(format!("Partition style: {}", if self.partitions_info.is_empty() { "Unknown" } else { &self.partitions_info[0].3 }));

                ui.separator();

                ui.heading("Partitions on this physical drive:");
                let partition_data: Vec<(u64, Color32, String)> = self.partitions_info
                    .iter()
                    .map(|(size, color, label, _style)| (*size, *color, label.clone()))
                    .collect();

                if let Some(disk_geometry) = &self.geometry {
                    let total_disk_size = unsafe { *disk_geometry.DiskSize.QuadPart() } as u64;
                    draw_partitions_bar(ui, &partition_data, total_disk_size);
                }

                ui.separator();
                ui.heading("Logical Drives on this physical drive:");
                for drive in &self.logical_drives_on_physical {
                    let drive_letter = drive.trim_end_matches('\\').trim_end_matches(':').to_string();
                    let is_selected = self.selected_logical_drive.as_ref() == Some(drive);

                    if ui.button(drive.clone()).clicked() {
                        self.selected_logical_drive = Some(drive.clone());
                        if let Some((total_gb, free_gb, used_gb)) = get_free_space(&drive_letter) {
                            self.drive_space_info = Some((total_gb, free_gb, used_gb));
                        } else {
                            self.drive_space_info = None;
                        }
                    }

                    if is_selected {
                        ui.label("Selected");
                    }
                }

                if let Some(drive) = &self.selected_logical_drive {
                    ui.separator();
                    ui.heading(format!("Logical Drive {} information:", drive));
                    if let Some((total_gb, free_gb, used_gb)) = &self.drive_space_info {
                        ui.label(format!("Total space: {:.1} GB", total_gb));
                        ui.label(format!("Free space: {:.1} GB", free_gb));
                        ui.label(format!("Used space: {:.1} GB", used_gb));
                        ui.horizontal(|ui| {
                            ui.label("Usage:");
                            let used_percent = (used_gb / total_gb) * 100.0;
                            ui.add(egui::ProgressBar::new(used_percent as f32 / 100.0).text(format!("{:.1}%", used_percent)));
                        });
                    } else {
                        ui.label("Failed to get disk space information.");
                    }
                }
            }
        });
    }
}

fn get_free_space(drive_letter: &str) -> Option<(f64, f64, f64)> {
    let path = format!("{}:\\", drive_letter);


    let path_utf16 = U16CString::from_str(&path).ok()?;

    let mut free_bytes_available: ULARGE_INTEGER = unsafe { std::mem::zeroed() };
    let mut total_number_of_bytes: ULARGE_INTEGER = unsafe { std::mem::zeroed() };
    let mut total_number_of_free_bytes: ULARGE_INTEGER = unsafe { std::mem::zeroed() };


    let result = unsafe {
        GetDiskFreeSpaceExW(
            path_utf16.as_ptr(),
            &mut free_bytes_available,
            &mut total_number_of_bytes,
            &mut total_number_of_free_bytes,
        )
    };

    if result == 0 {
        let error_code = unsafe { GetLastError() };
        None
    } else {
        let total_bytes = unsafe { *total_number_of_bytes.QuadPart() } as f64;
        let free_bytes = unsafe { *free_bytes_available.QuadPart() } as f64;
        let used_bytes = total_bytes - free_bytes;
        Some((
            total_bytes / (1024.0 * 1024.0 * 1024.0),
            free_bytes / (1024.0 * 1024.0 * 1024.0),
            used_bytes / (1024.0 * 1024.0 * 1024.0),
        ))
    }
}

fn get_partitions_on_physical_drive(index: usize) -> Vec<(u64, Color32, String, String)> {
    let mut partitions = Vec::new();
    let device_path = format!("\\\\.\\PHYSICALDRIVE{}", index);
    let device_path_utf16 = U16CString::from_str(&device_path).ok().unwrap();

    let handle = unsafe {
        CreateFileW(
            device_path_utf16.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null_mut(),
            winapi::um::fileapi::OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return partitions;
    }

    let mut layout_info: Vec<u8> = vec![0; std::mem::size_of::<DRIVE_LAYOUT_INFORMATION_EX>() + std::mem::size_of::<PARTITION_INFORMATION_EX>() * 128];
    let mut bytes_returned: u32 = 0;

    let result = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
            null_mut(),
            0,
            layout_info.as_mut_ptr() as *mut _,
            layout_info.len() as u32,
            &mut bytes_returned,
            null_mut(),
        )
    };

    if result != 0 {
        let layout_info = unsafe { &*(layout_info.as_ptr() as *const DRIVE_LAYOUT_INFORMATION_EX) };
        for i in 0..layout_info.PartitionCount {
            let partition_info = unsafe { &*(layout_info.PartitionEntry.as_ptr().add(i as usize) as *const PARTITION_INFORMATION_EX) };
            let size = unsafe { *partition_info.PartitionLength.QuadPart() as u64 };
            let color = get_partition_colors(partition_info.PartitionStyle as u8);
            let style = match partition_info.PartitionStyle {
                PARTITION_STYLE_MBR => "GPT",
                PARTITION_STYLE_GPT => "MBR",
                _ => "Unknown",
            };
            let label = format!("Partition {}", i + 1);
            partitions.push((size, color, label, style.to_string()));
        }
    }

    unsafe { CloseHandle(handle) };
    partitions
}


fn draw_partitions_bar(ui: &mut Ui, partitions: &[(u64, Color32, String)], total_disk_size: u64) {
    let min_partition_width = 60.0;

    let bar_height = 23.0;
    let text_height = 15.0;

    let (rect, _response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), bar_height + 130.0), egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);

        let mut start_x = rect.min.x;

        for (partition_size, color, file_system_type) in partitions {
            let width_ratio = (*partition_size as f32) / (total_disk_size as f32);
            let width = rect.width() * width_ratio;

            let effective_width = width.max(min_partition_width);

            if effective_width > 0.0 {
                let partition_rect = Rect::from_min_max(
                    Pos2::new(start_x, rect.min.y),
                    Pos2::new(start_x + effective_width, rect.min.y + bar_height),
                );


                painter.rect_filled(partition_rect, Rounding::none(), *color);
                painter.rect_stroke(partition_rect, Rounding::none(), Stroke::new(1.0, Color32::WHITE));


                let size_text = if *partition_size < 1024 * 1024 * 1024 {
                    format!("{:.1} MB", *partition_size as f64 / (1024.0 * 1024.0))
                } else {
                    format!("{:.1} GB", *partition_size as f64 / (1024.0 * 1024.0 * 1024.0))
                };


                let text_pos = Pos2::new(
                    start_x + effective_width / 2.0,
                    rect.min.y + bar_height / 2.0,
                );
                painter.text(
                    text_pos,
                    egui::Align2::CENTER_CENTER,
                    format!("{} ", size_text),
                    egui::FontId::proportional(10.0),
                    Color32::WHITE,
                );

                start_x += effective_width;
            }
        }


        let mut label_y = rect.min.y + bar_height + 10.0;

        for (partition_size, color, file_system_type) in partitions {
            let width_ratio = (*partition_size as f32) / (total_disk_size as f32);
            let percent = width_ratio * 100.0;

            let size_text = if *partition_size < 1024 * 1024 * 1024 {
                format!("{:.1} MB", *partition_size as f64 / (1024.0 * 1024.0))
            } else {
                format!("{:.1} GB", *partition_size as f64 / (1024.0 * 1024.0 * 1024.0))
            };


            painter.text(
                Pos2::new(rect.min.x, label_y),
                egui::Align2::LEFT_CENTER,
                format!("{:.1}% {} ({})", percent, file_system_type, size_text),
                egui::FontId::proportional(12.0),
                *color,
            );
            label_y += 20.0;
        }
    }
}

fn get_partition_colors(partition_type: u8) -> Color32 {
    match partition_type {
        0x07 => Color32::from_rgb(0, 120, 215),
        0x0B | 0x0C => Color32::from_rgb(0, 180, 0),
        0x27 => Color32::from_rgb(230, 230, 230),
        _ => Color32::from_rgb(128, 128, 128),
    }
}

fn main() -> Result<(), eframe::Error> {
    let app = HDDApp::default();
    let native_options = NativeOptions {
        initial_window_size: Some(egui::vec2(600.0, 560.0)),
        min_window_size: Some(egui::vec2(600.0, 560.0)),
        max_window_size: Some(egui::vec2(600.0, 560.0)),
        ..Default::default()
    };
    eframe::run_native(
        "plasitol's memory tools",
        native_options,
        Box::new(|_cc| Box::new(app)),
    )
}
