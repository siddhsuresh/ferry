//! PTP/MTP protocol constants — u16/u32 newtypes with spec-name `Display`.
//!
//! GENERATED, do not hand-edit. Regenerate from the Go reference with the
//! porting-scratchpad codegen. Faithful port of:
//!   * go-mtpfs `mtp/const.go` (munge.py output over libmtp `ptp.h`)
//!   * go-mtpfs `mtp/android.go` `init()` (OC 0x95C1..0x95C5 name additions)
//!
//! Every `PREFIX_Suffix` const becomes a SCREAMING_SNAKE associated const on
//! its group newtype; naming mirrors munge.py (strip `PTP_` + group prefix,
//! then camelCase -> SCREAMING_SNAKE). `name()` / [`code_name`] reproduce each
//! Go `PREFIX_names` map *exactly* — the vendor/duplicate-filtered debug subset
//! munge.py emits — so lookups match the Go stack byte-for-byte.
//!
//! Two documented ident deviations (values unchanged; leading digit / snake
//! collision are unrepresentable as distinct Rust idents):
//!   * `AT_2DPanoramic` (0x0006) -> [`AssociationType::PANORAMIC_2D`] (const.go:20).
//!   * `DPC_NIKON_ISO_Auto` (0xD16A) -> [`DevicePropCode::NIKON_ISO_AUTO_ALT`];
//!     its SCREAMING_SNAKE form collides with `DPC_NIKON_ISOAuto` (0xD054),
//!     a distinct libmtp `#define` at a different address (const.go:317/499).

#![allow(clippy::unreadable_literal)]

use core::fmt;

/// operation code (`OC_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct OpCode(pub u16);

impl OpCode {
    pub const UNDEFINED: Self = Self(0x1000);
    pub const GET_DEVICE_INFO: Self = Self(0x1001);
    pub const OPEN_SESSION: Self = Self(0x1002);
    pub const CLOSE_SESSION: Self = Self(0x1003);
    pub const GET_STORAGE_IDS: Self = Self(0x1004);
    pub const GET_STORAGE_INFO: Self = Self(0x1005);
    pub const GET_NUM_OBJECTS: Self = Self(0x1006);
    pub const GET_OBJECT_HANDLES: Self = Self(0x1007);
    pub const GET_OBJECT_INFO: Self = Self(0x1008);
    pub const GET_OBJECT: Self = Self(0x1009);
    pub const GET_THUMB: Self = Self(0x100A);
    pub const DELETE_OBJECT: Self = Self(0x100B);
    pub const SEND_OBJECT_INFO: Self = Self(0x100C);
    pub const SEND_OBJECT: Self = Self(0x100D);
    pub const INITIATE_CAPTURE: Self = Self(0x100E);
    pub const FORMAT_STORE: Self = Self(0x100F);
    pub const RESET_DEVICE: Self = Self(0x1010);
    pub const SELF_TEST: Self = Self(0x1011);
    pub const SET_OBJECT_PROTECTION: Self = Self(0x1012);
    pub const POWER_DOWN: Self = Self(0x1013);
    pub const GET_DEVICE_PROP_DESC: Self = Self(0x1014);
    pub const GET_DEVICE_PROP_VALUE: Self = Self(0x1015);
    pub const SET_DEVICE_PROP_VALUE: Self = Self(0x1016);
    pub const RESET_DEVICE_PROP_VALUE: Self = Self(0x1017);
    pub const TERMINATE_OPEN_CAPTURE: Self = Self(0x1018);
    pub const MOVE_OBJECT: Self = Self(0x1019);
    pub const COPY_OBJECT: Self = Self(0x101A);
    pub const GET_PARTIAL_OBJECT: Self = Self(0x101B);
    pub const INITIATE_OPEN_CAPTURE: Self = Self(0x101C);
    pub const START_ENUM_HANDLES: Self = Self(0x101D);
    pub const ENUM_HANDLES: Self = Self(0x101E);
    pub const STOP_ENUM_HANDLES: Self = Self(0x101F);
    pub const GET_VENDOR_EXTENSION_MAPS: Self = Self(0x1020);
    pub const GET_VENDOR_DEVICE_INFO: Self = Self(0x1021);
    pub const GET_RESIZED_IMAGE_OBJECT: Self = Self(0x1022);
    pub const GET_FILESYSTEM_MANIFEST: Self = Self(0x1023);
    pub const GET_STREAM_INFO: Self = Self(0x1024);
    pub const GET_STREAM: Self = Self(0x1025);
    pub const EXTENSION: Self = Self(0x9000);
    pub const CANON_GET_PARTIAL_OBJECT_INFO: Self = Self(0x9001);
    pub const CASIO_STILL_START: Self = Self(0x9001);
    pub const CANON_SET_OBJECT_ARCHIVE: Self = Self(0x9002);
    pub const CASIO_STILL_STOP: Self = Self(0x9002);
    pub const CANON_KEEP_DEVICE_ON: Self = Self(0x9003);
    pub const EK_GET_SERIAL: Self = Self(0x9003);
    pub const CANON_LOCK_DEVICE_UI: Self = Self(0x9004);
    pub const EK_SET_SERIAL: Self = Self(0x9004);
    pub const CANON_UNLOCK_DEVICE_UI: Self = Self(0x9005);
    pub const EK_SEND_FILE_OBJECT_INFO: Self = Self(0x9005);
    pub const CANON_GET_OBJECT_HANDLE_BY_NAME: Self = Self(0x9006);
    pub const EK_SEND_FILE_OBJECT: Self = Self(0x9006);
    pub const NIKON_GET_PROFILE_ALL_DATA: Self = Self(0x9006);
    pub const CASIO_FOCUS: Self = Self(0x9007);
    pub const NIKON_SEND_PROFILE_DATA: Self = Self(0x9007);
    pub const CANON_INITIATE_RELEASE_CONTROL: Self = Self(0x9008);
    pub const EK_SET_TEXT: Self = Self(0x9008);
    pub const NIKON_DELETE_PROFILE: Self = Self(0x9008);
    pub const CANON_TERMINATE_RELEASE_CONTROL: Self = Self(0x9009);
    pub const CASIO_CF_PRESS: Self = Self(0x9009);
    pub const NIKON_SET_PROFILE_DATA: Self = Self(0x9009);
    pub const CANON_TERMINATE_PLAYBACK_MODE: Self = Self(0x900A);
    pub const CASIO_CF_RELEASE: Self = Self(0x900A);
    pub const CANON_VIEWFINDER_ON: Self = Self(0x900B);
    pub const CANON_VIEWFINDER_OFF: Self = Self(0x900C);
    pub const CASIO_GET_OBJECT_INFO: Self = Self(0x900C);
    pub const CANON_DO_AE_AF_AWB: Self = Self(0x900D);
    pub const CANON_GET_CUSTOMIZE_SPEC: Self = Self(0x900E);
    pub const CANON_GET_CUSTOMIZE_ITEM_INFO: Self = Self(0x900F);
    pub const CANON_GET_CUSTOMIZE_DATA: Self = Self(0x9010);
    pub const NIKON_ADVANCED_TRANSFER: Self = Self(0x9010);
    pub const CANON_SET_CUSTOMIZE_DATA: Self = Self(0x9011);
    pub const NIKON_GET_FILE_INFO_IN_BLOCK: Self = Self(0x9011);
    pub const CANON_GET_CAPTURE_STATUS: Self = Self(0x9012);
    pub const CANON_CHECK_EVENT: Self = Self(0x9013);
    pub const CANON_FOCUS_LOCK: Self = Self(0x9014);
    pub const CANON_FOCUS_UNLOCK: Self = Self(0x9015);
    pub const CANON_GET_LOCAL_RELEASE_PARAM: Self = Self(0x9016);
    pub const CANON_SET_LOCAL_RELEASE_PARAM: Self = Self(0x9017);
    pub const CANON_ASK_ABOUT_PC_EVF: Self = Self(0x9018);
    pub const CANON_SEND_PARTIAL_OBJECT: Self = Self(0x9019);
    pub const CANON_INITIATE_CAPTURE_IN_MEMORY: Self = Self(0x901A);
    pub const CANON_GET_PARTIAL_OBJECT_EX: Self = Self(0x901B);
    pub const CANON_SET_OBJECT_TIME: Self = Self(0x901C);
    pub const CANON_GET_VIEWFINDER_IMAGE: Self = Self(0x901D);
    pub const CANON_GET_OBJECT_ATTRIBUTES: Self = Self(0x901E);
    pub const CANON_CHANGE_USB_PROTOCOL: Self = Self(0x901F);
    pub const CANON_GET_CHANGES: Self = Self(0x9020);
    pub const CANON_GET_OBJECT_INFO_EX: Self = Self(0x9021);
    pub const CANON_INITIATE_DIRECT_TRANSFER: Self = Self(0x9022);
    pub const CANON_TERMINATE_DIRECT_TRANSFER: Self = Self(0x9023);
    pub const CANON_SEND_OBJECT_INFO_BY_PATH: Self = Self(0x9024);
    pub const CASIO_SHUTTER: Self = Self(0x9024);
    pub const CANON_SEND_OBJECT_BY_PATH: Self = Self(0x9025);
    pub const CASIO_GET_OBJECT: Self = Self(0x9025);
    pub const CANON_INITIATE_DIRECT_TANSFER_EX: Self = Self(0x9026);
    pub const CASIO_GET_THUMBNAIL: Self = Self(0x9026);
    pub const CANON_GET_ANCILLARY_OBJECT_HANDLES: Self = Self(0x9027);
    pub const CASIO_GET_STILL_HANDLES: Self = Self(0x9027);
    pub const CANON_GET_TREE_INFO: Self = Self(0x9028);
    pub const CASIO_STILL_RESET: Self = Self(0x9028);
    pub const CANON_GET_TREE_SIZE: Self = Self(0x9029);
    pub const CASIO_HALF_PRESS: Self = Self(0x9029);
    pub const CANON_NOTIFY_PROGRESS: Self = Self(0x902A);
    pub const CASIO_HALF_RELEASE: Self = Self(0x902A);
    pub const CANON_NOTIFY_CANCEL_ACCEPTED: Self = Self(0x902B);
    pub const CASIO_CS_PRESS: Self = Self(0x902B);
    pub const CANON_902C: Self = Self(0x902C);
    pub const CASIO_CS_RELEASE: Self = Self(0x902C);
    pub const CANON_GET_DIRECTORY: Self = Self(0x902D);
    pub const CASIO_ZOOM: Self = Self(0x902D);
    pub const CASIO_CZ_PRESS: Self = Self(0x902E);
    pub const CASIO_CZ_RELEASE: Self = Self(0x902F);
    pub const CANON_SET_PAIRING_INFO: Self = Self(0x9030);
    pub const CANON_GET_PAIRING_INFO: Self = Self(0x9031);
    pub const CANON_DELETE_PAIRING_INFO: Self = Self(0x9032);
    pub const CANON_GET_MAC_ADDRESS: Self = Self(0x9033);
    pub const CANON_SET_DISPLAY_MONITOR: Self = Self(0x9034);
    pub const CANON_PAIRING_COMPLETE: Self = Self(0x9035);
    pub const CANON_GET_WIRELESS_MAX_CHANNEL: Self = Self(0x9036);
    pub const CASIO_MOVIE_START: Self = Self(0x9041);
    pub const CASIO_MOVIE_STOP: Self = Self(0x9042);
    pub const CASIO_MOVIE_PRESS: Self = Self(0x9043);
    pub const CASIO_MOVIE_RELEASE: Self = Self(0x9044);
    pub const CASIO_GET_MOVIE_HANDLES: Self = Self(0x9045);
    pub const CASIO_MOVIE_RESET: Self = Self(0x9046);
    pub const NIKON_GET_LARGE_THUMB: Self = Self(0x90C4);
    pub const NIKON_GET_PICT_CTRL_DATA: Self = Self(0x90CC);
    pub const NIKON_SET_PICT_CTRL_DATA: Self = Self(0x90CD);
    pub const NIKON_DEL_CST_PIC_CTRL: Self = Self(0x90CE);
    pub const NIKON_GET_PIC_CTRL_CAPABILITY: Self = Self(0x90CF);
    pub const NIKON_GET_DEVICE_PTPIP_INFO: Self = Self(0x90E0);
    pub const CANON_EOS_GET_STORAGE_IDS: Self = Self(0x9101);
    pub const MTP_WMDRMPD_GET_SECURE_TIME_CHALLENGE: Self = Self(0x9101);
    pub const OLYMPUS_CAPTURE: Self = Self(0x9101);
    pub const CANON_EOS_GET_STORAGE_INFO: Self = Self(0x9102);
    pub const MTP_WMDRMPD_GET_SECURE_TIME_RESPONSE: Self = Self(0x9102);
    pub const CANON_EOS_GET_OBJECT_INFO: Self = Self(0x9103);
    pub const MTP_WMDRMPD_SET_LICENSE_RESPONSE: Self = Self(0x9103);
    pub const OLYMPUS_SELF_CLEANING: Self = Self(0x9103);
    pub const CANON_EOS_GET_OBJECT: Self = Self(0x9104);
    pub const MTP_WMDRMPD_GET_SYNC_LIST: Self = Self(0x9104);
    pub const CANON_EOS_DELETE_OBJECT: Self = Self(0x9105);
    pub const MTP_WMDRMPD_SEND_METER_CHALLENGE_QUERY: Self = Self(0x9105);
    pub const CANON_EOS_FORMAT_STORE: Self = Self(0x9106);
    pub const MTP_WMDRMPD_GET_METER_CHALLENGE: Self = Self(0x9106);
    pub const OLYMPUS_SET_RGB_GAIN: Self = Self(0x9106);
    pub const CANON_EOS_GET_PARTIAL_OBJECT: Self = Self(0x9107);
    pub const MTP_WMDRMPD_SET_METER_RESPONSE: Self = Self(0x9107);
    pub const OLYMPUS_SET_PRESET_MODE: Self = Self(0x9107);
    pub const CANON_EOS_GET_DEVICE_INFO_EX: Self = Self(0x9108);
    pub const MTP_WMDRMPD_CLEAN_DATA_STORE: Self = Self(0x9108);
    pub const OLYMPUS_SET_WB_BIAS_ALL: Self = Self(0x9108);
    pub const CANON_EOS_GET_OBJECT_INFO_EX: Self = Self(0x9109);
    pub const MTP_WMDRMPD_GET_LICENSE_STATE: Self = Self(0x9109);
    pub const CANON_EOS_GET_THUMB_EX: Self = Self(0x910A);
    pub const MTP_WMDRMPD_SEND_WMDRMPD_COMMAND: Self = Self(0x910A);
    pub const CANON_EOS_SEND_PARTIAL_OBJECT: Self = Self(0x910B);
    pub const MTP_WMDRMPD_SEND_WMDRMPD_REQUEST: Self = Self(0x910B);
    pub const CANON_EOS_SET_OBJECT_ATTRIBUTES: Self = Self(0x910C);
    pub const CANON_EOS_GET_OBJECT_TIME: Self = Self(0x910D);
    pub const CANON_EOS_SET_OBJECT_TIME: Self = Self(0x910E);
    pub const CANON_EOS_REMOTE_RELEASE: Self = Self(0x910F);
    pub const OLYMPUS_GET_CAMERA_CONTROL_MODE: Self = Self(0x910A);
    pub const OLYMPUS_SET_CAMERA_CONTROL_MODE: Self = Self(0x910B);
    pub const OLYMPUS_SET_WBRGB_GAIN: Self = Self(0x910C);
    pub const CANON_EOS_SET_DEVICE_PROP_VALUE_EX: Self = Self(0x9110);
    pub const CANON_EOS_GET_REMOTE_MODE: Self = Self(0x9113);
    pub const CANON_EOS_SET_REMOTE_MODE: Self = Self(0x9114);
    pub const CANON_EOS_SET_EVENT_MODE: Self = Self(0x9115);
    pub const CANON_EOS_GET_EVENT: Self = Self(0x9116);
    pub const CANON_EOS_TRANSFER_COMPLETE: Self = Self(0x9117);
    pub const CANON_EOS_CANCEL_TRANSFER: Self = Self(0x9118);
    pub const CANON_EOS_RESET_TRANSFER: Self = Self(0x9119);
    pub const CANON_EOS_PCHDD_CAPACITY: Self = Self(0x911A);
    pub const CANON_EOS_SET_UI_LOCK: Self = Self(0x911B);
    pub const CANON_EOS_RESET_UI_LOCK: Self = Self(0x911C);
    pub const CANON_EOS_KEEP_DEVICE_ON: Self = Self(0x911D);
    pub const CANON_EOS_SET_NULL_PACKET_MODE: Self = Self(0x911E);
    pub const CANON_EOS_UPDATE_FIRMWARE: Self = Self(0x911F);
    pub const CANON_EOS_TRANSFER_COMPLETE_DT: Self = Self(0x9120);
    pub const CANON_EOS_CANCEL_TRANSFER_DT: Self = Self(0x9121);
    pub const CANON_EOS_GET_WFT_PROFILE: Self = Self(0x9122);
    pub const CANON_EOS_SET_WFT_PROFILE: Self = Self(0x9122);
    pub const MTP_WPDWCN_PROCESS_WFC_OBJECT: Self = Self(0x9122);
    pub const CANON_EOS_SET_PROFILE_TO_WFT: Self = Self(0x9124);
    pub const CANON_EOS_BULB_START: Self = Self(0x9125);
    pub const CANON_EOS_BULB_END: Self = Self(0x9126);
    pub const CANON_EOS_REQUEST_DEVICE_PROP_VALUE: Self = Self(0x9127);
    pub const CANON_EOS_REMOTE_RELEASE_ON: Self = Self(0x9128);
    pub const CANON_EOS_REMOTE_RELEASE_OFF: Self = Self(0x9129);
    pub const CANON_EOS_INITIATE_VIEWFINDER: Self = Self(0x9151);
    pub const CANON_EOS_TERMINATE_VIEWFINDER: Self = Self(0x9152);
    pub const CANON_EOS_GET_VIEW_FINDER_DATA: Self = Self(0x9153);
    pub const CANON_EOS_DO_AF: Self = Self(0x9154);
    pub const CANON_EOS_DRIVE_LENS: Self = Self(0x9155);
    pub const CANON_EOS_DEPTH_OF_FIELD_PREVIEW: Self = Self(0x9156);
    pub const CANON_EOS_CLICK_WB: Self = Self(0x9157);
    pub const CANON_EOS_ZOOM: Self = Self(0x9158);
    pub const CANON_EOS_ZOOM_POSITION: Self = Self(0x9159);
    pub const CANON_EOS_SET_LIVE_AF_FRAME: Self = Self(0x915A);
    pub const CANON_EOS_AF_CANCEL: Self = Self(0x9160);
    pub const MTP_AAVT_OPEN_MEDIA_SESSION: Self = Self(0x9170);
    pub const MTP_AAVT_CLOSE_MEDIA_SESSION: Self = Self(0x9171);
    pub const MTP_AAVT_GET_NEXT_DATA_BLOCK: Self = Self(0x9172);
    pub const MTP_AAVT_SET_CURRENT_TIME_POSITION: Self = Self(0x9173);
    pub const MTP_WMDRMND_SEND_REGISTRATION_REQUEST: Self = Self(0x9180);
    pub const MTP_WMDRMND_GET_REGISTRATION_RESPONSE: Self = Self(0x9181);
    pub const MTP_WMDRMND_GET_PROXIMITY_CHALLENGE: Self = Self(0x9182);
    pub const MTP_WMDRMND_SEND_PROXIMITY_RESPONSE: Self = Self(0x9183);
    pub const MTP_WMDRMND_SEND_WMDRMND_LICENSE_REQUEST: Self = Self(0x9184);
    pub const MTP_WMDRMND_GET_WMDRMND_LICENSE_RESPONSE: Self = Self(0x9185);
    pub const CANON_EOS_FAPI_MESSAGE_TX: Self = Self(0x91FE);
    pub const CANON_EOS_FAPI_MESSAGE_RX: Self = Self(0x91FF);
    pub const NIKON_GET_PREVIEW_IMG: Self = Self(0x9200);
    pub const MTP_WMPPD_REPORT_ADDED_DELETED_ITEMS: Self = Self(0x9201);
    pub const NIKON_START_LIVE_VIEW: Self = Self(0x9201);
    pub const MTP_WMPPD_REPORT_ACQUIRED_ITEMS: Self = Self(0x9202);
    pub const NIKON_END_LIVE_VIEW: Self = Self(0x9202);
    pub const MTP_WMPPD_PLAYLIST_OBJECT_PREF: Self = Self(0x9203);
    pub const NIKON_GET_LIVE_VIEW_IMG: Self = Self(0x9203);
    pub const MTP_ZUNE_GETUNDEFINED001: Self = Self(0x9204);
    pub const NIKON_MF_DRIVE: Self = Self(0x9204);
    pub const NIKON_CHANGE_AF_AREA: Self = Self(0x9205);
    pub const NIKON_AF_DRIVE_CANCEL: Self = Self(0x9206);
    pub const MTP_WMDRMPD_SEND_WMDRMPD_APP_REQUEST: Self = Self(0x9212);
    pub const MTP_WMDRMPD_GET_WMDRMPD_APP_RESPONSE: Self = Self(0x9213);
    pub const MTP_WMDRMPD_ENABLE_TRUSTED_FILES_OPERATIONS: Self = Self(0x9214);
    pub const MTP_WMDRMPD_DISABLE_TRUSTED_FILES_OPERATIONS: Self = Self(0x9215);
    pub const MTP_WMDRMPD_END_TRUSTED_APP_SESSION: Self = Self(0x9216);
    pub const OLYMPUS_GET_DEVICE_INFO: Self = Self(0x9301);
    pub const OLYMPUS_INIT1: Self = Self(0x9302);
    pub const OLYMPUS_SET_DATE_TIME: Self = Self(0x9402);
    pub const OLYMPUS_GET_DATE_TIME: Self = Self(0x9482);
    pub const OLYMPUS_SET_CAMERA_ID: Self = Self(0x9501);
    pub const OLYMPUS_GET_CAMERA_ID: Self = Self(0x9581);
    pub const MTP_GET_OBJECT_PROPS_SUPPORTED: Self = Self(0x9801);
    pub const MTP_GET_OBJECT_PROP_DESC: Self = Self(0x9802);
    pub const MTP_GET_OBJECT_PROP_VALUE: Self = Self(0x9803);
    pub const MTP_SET_OBJECT_PROP_VALUE: Self = Self(0x9804);
    pub const MTP_GET_OBJ_PROP_LIST: Self = Self(0x9805);
    pub const MTP_SET_OBJ_PROP_LIST: Self = Self(0x9806);
    pub const MTP_GET_INTERDEPENDEND_PROPDESC: Self = Self(0x9807);
    pub const MTP_SEND_OBJECT_PROP_LIST: Self = Self(0x9808);
    pub const MTP_GET_OBJECT_REFERENCES: Self = Self(0x9810);
    pub const MTP_SET_OBJECT_REFERENCES: Self = Self(0x9811);
    pub const MTP_UPDATE_DEVICE_FIRMWARE: Self = Self(0x9812);
    pub const MTP_SKIP: Self = Self(0x9820);
    pub const CHDK: Self = Self(0x9999);
    pub const EXTENSION_MASK: Self = Self(0xF000);
    pub const ANDROID_GET_PARTIAL_OBJECT64: Self = Self(0x95C1);
    pub const ANDROID_SEND_PARTIAL_OBJECT: Self = Self(0x95C2);
    pub const ANDROID_TRUNCATE_OBJECT: Self = Self(0x95C3);
    pub const ANDROID_BEGIN_EDIT_OBJECT: Self = Self(0x95C4);
    pub const ANDROID_END_EDIT_OBJECT: Self = Self(0x95C5);

    /// Debug name from go-mtpfs `OC_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x1000 => "Undefined",
            0x1001 => "GetDeviceInfo",
            0x1002 => "OpenSession",
            0x1003 => "CloseSession",
            0x1004 => "GetStorageIDs",
            0x1005 => "GetStorageInfo",
            0x1006 => "GetNumObjects",
            0x1007 => "GetObjectHandles",
            0x1008 => "GetObjectInfo",
            0x1009 => "GetObject",
            0x100A => "GetThumb",
            0x100B => "DeleteObject",
            0x100C => "SendObjectInfo",
            0x100D => "SendObject",
            0x100E => "InitiateCapture",
            0x100F => "FormatStore",
            0x1010 => "ResetDevice",
            0x1011 => "SelfTest",
            0x1012 => "SetObjectProtection",
            0x1013 => "PowerDown",
            0x1014 => "GetDevicePropDesc",
            0x1015 => "GetDevicePropValue",
            0x1016 => "SetDevicePropValue",
            0x1017 => "ResetDevicePropValue",
            0x1018 => "TerminateOpenCapture",
            0x1019 => "MoveObject",
            0x101A => "CopyObject",
            0x101B => "GetPartialObject",
            0x101C => "InitiateOpenCapture",
            0x101D => "StartEnumHandles",
            0x101E => "EnumHandles",
            0x101F => "StopEnumHandles",
            0x1020 => "GetVendorExtensionMaps",
            0x1021 => "GetVendorDeviceInfo",
            0x1022 => "GetResizedImageObject",
            0x1023 => "GetFilesystemManifest",
            0x1024 => "GetStreamInfo",
            0x1025 => "GetStream",
            0x9000 => "EXTENSION",
            0x9007 => "CASIO_FOCUS",
            0x902E => "CASIO_CZ_PRESS",
            0x902F => "CASIO_CZ_RELEASE",
            0x9041 => "CASIO_MOVIE_START",
            0x9042 => "CASIO_MOVIE_STOP",
            0x9043 => "CASIO_MOVIE_PRESS",
            0x9044 => "CASIO_MOVIE_RELEASE",
            0x9045 => "CASIO_GET_MOVIE_HANDLES",
            0x9046 => "CASIO_MOVIE_RESET",
            0x9170 => "MTP_AAVT_OpenMediaSession",
            0x9171 => "MTP_AAVT_CloseMediaSession",
            0x9172 => "MTP_AAVT_GetNextDataBlock",
            0x9173 => "MTP_AAVT_SetCurrentTimePosition",
            0x9180 => "MTP_WMDRMND_SendRegistrationRequest",
            0x9181 => "MTP_WMDRMND_GetRegistrationResponse",
            0x9182 => "MTP_WMDRMND_GetProximityChallenge",
            0x9183 => "MTP_WMDRMND_SendProximityResponse",
            0x9184 => "MTP_WMDRMND_SendWMDRMNDLicenseRequest",
            0x9185 => "MTP_WMDRMND_GetWMDRMNDLicenseResponse",
            0x9201 => "MTP_WMPPD_ReportAddedDeletedItems",
            0x9202 => "MTP_WMPPD_ReportAcquiredItems",
            0x9203 => "MTP_WMPPD_PlaylistObjectPref",
            0x9204 => "MTP_ZUNE_GETUNDEFINED001",
            0x9212 => "MTP_WMDRMPD_SendWMDRMPDAppRequest",
            0x9213 => "MTP_WMDRMPD_GetWMDRMPDAppResponse",
            0x9214 => "MTP_WMDRMPD_EnableTrustedFilesOperations",
            0x9215 => "MTP_WMDRMPD_DisableTrustedFilesOperations",
            0x9216 => "MTP_WMDRMPD_EndTrustedAppSession",
            0x9301 => "OLYMPUS_GetDeviceInfo",
            0x9302 => "OLYMPUS_Init1",
            0x9402 => "OLYMPUS_SetDateTime",
            0x9482 => "OLYMPUS_GetDateTime",
            0x9501 => "OLYMPUS_SetCameraID",
            0x9581 => "OLYMPUS_GetCameraID",
            0x9801 => "MTP_GetObjectPropsSupported",
            0x9802 => "MTP_GetObjectPropDesc",
            0x9803 => "MTP_GetObjectPropValue",
            0x9804 => "MTP_SetObjectPropValue",
            0x9805 => "MTP_GetObjPropList",
            0x9806 => "MTP_SetObjPropList",
            0x9807 => "MTP_GetInterdependendPropdesc",
            0x9808 => "MTP_SendObjectPropList",
            0x9810 => "MTP_GetObjectReferences",
            0x9811 => "MTP_SetObjectReferences",
            0x9812 => "MTP_UpdateDeviceFirmware",
            0x9820 => "MTP_Skip",
            0x9999 => "CHDK",
            0xF000 => "EXTENSION_MASK",
            0x95C1 => "ANDROID_GET_PARTIAL_OBJECT64",
            0x95C2 => "ANDROID_SEND_PARTIAL_OBJECT",
            0x95C3 => "ANDROID_TRUNCATE_OBJECT",
            0x95C4 => "ANDROID_BEGIN_EDIT_OBJECT",
            0x95C5 => "ANDROID_END_EDIT_OBJECT",
            _ => return None,
        })
    }
}

impl fmt::Display for OpCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// return code (`RC_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct RespCode(pub u16);

impl RespCode {
    pub const UNDEFINED: Self = Self(0x2000);
    pub const OK: Self = Self(0x2001);
    pub const GENERAL_ERROR: Self = Self(0x2002);
    pub const SESSION_NOT_OPEN: Self = Self(0x2003);
    pub const INVALID_TRANSACTION_ID: Self = Self(0x2004);
    pub const OPERATION_NOT_SUPPORTED: Self = Self(0x2005);
    pub const PARAMETER_NOT_SUPPORTED: Self = Self(0x2006);
    pub const INCOMPLETE_TRANSFER: Self = Self(0x2007);
    pub const INVALID_STORAGE_ID: Self = Self(0x2008);
    pub const INVALID_OBJECT_HANDLE: Self = Self(0x2009);
    pub const DEVICE_PROP_NOT_SUPPORTED: Self = Self(0x200A);
    pub const INVALID_OBJECT_FORMAT_CODE: Self = Self(0x200B);
    pub const STORE_FULL: Self = Self(0x200C);
    pub const OBJECT_WRITE_PROTECTED: Self = Self(0x200D);
    pub const STORE_READ_ONLY: Self = Self(0x200E);
    pub const ACCESS_DENIED: Self = Self(0x200F);
    pub const NO_THUMBNAIL_PRESENT: Self = Self(0x2010);
    pub const SELF_TEST_FAILED: Self = Self(0x2011);
    pub const PARTIAL_DELETION: Self = Self(0x2012);
    pub const STORE_NOT_AVAILABLE: Self = Self(0x2013);
    pub const SPECIFICATION_BY_FORMAT_UNSUPPORTED: Self = Self(0x2014);
    pub const NO_VALID_OBJECT_INFO: Self = Self(0x2015);
    pub const INVALID_CODE_FORMAT: Self = Self(0x2016);
    pub const UNKNOWN_VENDOR_CODE: Self = Self(0x2017);
    pub const CAPTURE_ALREADY_TERMINATED: Self = Self(0x2018);
    pub const DEVICE_BUSY: Self = Self(0x2019);
    pub const INVALID_PARENT_OBJECT: Self = Self(0x201A);
    pub const INVALID_DEVICE_PROP_FORMAT: Self = Self(0x201B);
    pub const INVALID_DEVICE_PROP_VALUE: Self = Self(0x201C);
    pub const INVALID_PARAMETER: Self = Self(0x201D);
    pub const SESSION_ALREADY_OPENED: Self = Self(0x201E);
    pub const TRANSACTION_CANCELED: Self = Self(0x201F);
    pub const SPECIFICATION_OF_DESTINATION_UNSUPPORTED: Self = Self(0x2020);
    pub const INVALID_ENUM_HANDLE: Self = Self(0x2021);
    pub const NO_STREAM_ENABLED: Self = Self(0x2022);
    pub const INVALID_DATA_SET: Self = Self(0x2023);
    pub const CANON_UNKNOWN_COMMAND: Self = Self(0xA001);
    pub const EK_FILENAME_REQUIRED: Self = Self(0xA001);
    pub const NIKON_HARDWARE_ERROR: Self = Self(0xA001);
    pub const EK_FILENAME_CONFLICTS: Self = Self(0xA002);
    pub const NIKON_OUT_OF_FOCUS: Self = Self(0xA002);
    pub const EK_FILENAME_INVALID: Self = Self(0xA003);
    pub const NIKON_CHANGE_CAMERA_MODE_FAILED: Self = Self(0xA003);
    pub const NIKON_INVALID_STATUS: Self = Self(0xA004);
    pub const CANON_OPERATION_REFUSED: Self = Self(0xA005);
    pub const NIKON_SET_PROPERTY_NOT_SUPPORTED: Self = Self(0xA005);
    pub const CANON_LENS_COVER: Self = Self(0xA006);
    pub const NIKON_WB_RESET_ERROR: Self = Self(0xA006);
    pub const NIKON_DUST_REFERENCE_ERROR: Self = Self(0xA007);
    pub const NIKON_SHUTTER_SPEED_BULB: Self = Self(0xA008);
    pub const CANON_A009: Self = Self(0xA009);
    pub const NIKON_MIRROR_UP_SEQUENCE: Self = Self(0xA009);
    pub const NIKON_CAMERA_MODE_NOT_ADJUST_F_NUMBER: Self = Self(0xA00A);
    pub const NIKON_NOT_LIVE_VIEW: Self = Self(0xA00B);
    pub const NIKON_MF_DRIVE_STEP_END: Self = Self(0xA00C);
    pub const NIKON_MF_DRIVE_STEP_INSUFFICIENCY: Self = Self(0xA00E);
    pub const NIKON_ADVANCED_TRANSFER_CANCEL: Self = Self(0xA022);
    pub const CANON_BATTERY_LOW: Self = Self(0xA101);
    pub const CANON_NOT_READY: Self = Self(0xA102);
    pub const MTP_INVALID_WFC_SYNTAX: Self = Self(0xA121);
    pub const MTP_WFC_VERSION_NOT_SUPPORTED: Self = Self(0xA122);
    pub const MTP_MEDIA_SESSION_LIMIT_REACHED: Self = Self(0xA171);
    pub const MTP_NO_MORE_DATA: Self = Self(0xA172);
    pub const MTP_UNDEFINED: Self = Self(0xA800);
    pub const MTP_INVALID_OBJECT_PROP_CODE: Self = Self(0xA801);
    pub const MTP_INVALID_OBJECT_PROP_FORMAT: Self = Self(0xA802);
    pub const MTP_INVALID_OBJECT_PROP_VALUE: Self = Self(0xA803);
    pub const MTP_INVALID_OBJECT_REFERENCE: Self = Self(0xA804);
    pub const MTP_INVALID_DATASET: Self = Self(0xA806);
    pub const MTP_SPECIFICATION_BY_GROUP_UNSUPPORTED: Self = Self(0xA807);
    pub const MTP_SPECIFICATION_BY_DEPTH_UNSUPPORTED: Self = Self(0xA808);
    pub const MTP_OBJECT_TOO_LARGE: Self = Self(0xA809);
    pub const MTP_OBJECT_PROP_NOT_SUPPORTED: Self = Self(0xA80A);

    /// Debug name from go-mtpfs `RC_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x2000 => "Undefined",
            0x2001 => "OK",
            0x2002 => "GeneralError",
            0x2003 => "SessionNotOpen",
            0x2004 => "InvalidTransactionID",
            0x2005 => "OperationNotSupported",
            0x2006 => "ParameterNotSupported",
            0x2007 => "IncompleteTransfer",
            0x2008 => "InvalidStorageId",
            0x2009 => "InvalidObjectHandle",
            0x200A => "DevicePropNotSupported",
            0x200B => "InvalidObjectFormatCode",
            0x200C => "StoreFull",
            0x200D => "ObjectWriteProtected",
            0x200E => "StoreReadOnly",
            0x200F => "AccessDenied",
            0x2010 => "NoThumbnailPresent",
            0x2011 => "SelfTestFailed",
            0x2012 => "PartialDeletion",
            0x2013 => "StoreNotAvailable",
            0x2014 => "SpecificationByFormatUnsupported",
            0x2015 => "NoValidObjectInfo",
            0x2016 => "InvalidCodeFormat",
            0x2017 => "UnknownVendorCode",
            0x2018 => "CaptureAlreadyTerminated",
            0x2019 => "DeviceBusy",
            0x201A => "InvalidParentObject",
            0x201B => "InvalidDevicePropFormat",
            0x201C => "InvalidDevicePropValue",
            0x201D => "InvalidParameter",
            0x201E => "SessionAlreadyOpened",
            0x201F => "TransactionCanceled",
            0x2020 => "SpecificationOfDestinationUnsupported",
            0x2021 => "InvalidEnumHandle",
            0x2022 => "NoStreamEnabled",
            0x2023 => "InvalidDataSet",
            0xA121 => "MTP_Invalid_WFC_Syntax",
            0xA122 => "MTP_WFC_Version_Not_Supported",
            0xA171 => "MTP_Media_Session_Limit_Reached",
            0xA172 => "MTP_No_More_Data",
            0xA800 => "MTP_Undefined",
            0xA801 => "MTP_Invalid_ObjectPropCode",
            0xA802 => "MTP_Invalid_ObjectProp_Format",
            0xA803 => "MTP_Invalid_ObjectProp_Value",
            0xA804 => "MTP_Invalid_ObjectReference",
            0xA806 => "MTP_Invalid_Dataset",
            0xA807 => "MTP_Specification_By_Group_Unsupported",
            0xA808 => "MTP_Specification_By_Depth_Unsupported",
            0xA809 => "MTP_Object_Too_Large",
            0xA80A => "MTP_ObjectProp_Not_Supported",
            _ => return None,
        })
    }
}

impl fmt::Display for RespCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// event code (`EC_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct EventCode(pub u16);

impl EventCode {
    pub const UNDEFINED: Self = Self(0x4000);
    pub const CANCEL_TRANSACTION: Self = Self(0x4001);
    pub const OBJECT_ADDED: Self = Self(0x4002);
    pub const OBJECT_REMOVED: Self = Self(0x4003);
    pub const STORE_ADDED: Self = Self(0x4004);
    pub const STORE_REMOVED: Self = Self(0x4005);
    pub const DEVICE_PROP_CHANGED: Self = Self(0x4006);
    pub const OBJECT_INFO_CHANGED: Self = Self(0x4007);
    pub const DEVICE_INFO_CHANGED: Self = Self(0x4008);
    pub const REQUEST_OBJECT_TRANSFER: Self = Self(0x4009);
    pub const STORE_FULL: Self = Self(0x400A);
    pub const DEVICE_RESET: Self = Self(0x400B);
    pub const STORAGE_INFO_CHANGED: Self = Self(0x400C);
    pub const CAPTURE_COMPLETE: Self = Self(0x400D);
    pub const UNREPORTED_STATUS: Self = Self(0x400E);
    pub const CANON_OBJECT_INFO_CHANGED: Self = Self(0xC008);
    pub const CANON_REQUEST_OBJECT_TRANSFER: Self = Self(0xC009);
    pub const CANON_CAMERA_MODE_CHANGED: Self = Self(0xC00C);
    pub const CANON_SHUTTER_BUTTON_PRESSED: Self = Self(0xC00E);
    pub const CANON_START_DIRECT_TRANSFER: Self = Self(0xC011);
    pub const CANON_STOP_DIRECT_TRANSFER: Self = Self(0xC013);
    pub const NIKON_OBJECT_ADDED_IN_SDRAM: Self = Self(0xC101);
    pub const NIKON_CAPTURE_COMPLETE_REC_IN_SDRAM: Self = Self(0xC102);
    pub const NIKON_ADVANCED_TRANSFER: Self = Self(0xC103);
    pub const NIKON_PREVIEW_IMAGE_ADDED: Self = Self(0xC104);
    pub const CANON_EOS_REQUEST_OBJECT_TRANSFER_TS: Self = Self(0xC1A2);
    pub const MTP_OBJECT_PROP_CHANGED: Self = Self(0xC801);
    pub const MTP_OBJECT_PROP_DESC_CHANGED: Self = Self(0xC802);
    pub const MTP_OBJECT_REFERENCES_CHANGED: Self = Self(0xC803);
    pub const CANON_EOS_REQUEST_GET_EVENT: Self = Self(0xC101);
    pub const CANON_EOS_OBJECT_ADDED_EX: Self = Self(0xC181);
    pub const CANON_EOS_OBJECT_REMOVED: Self = Self(0xC182);
    pub const CANON_EOS_REQUEST_GET_OBJECT_INFO_EX: Self = Self(0xC183);
    pub const CANON_EOS_STORAGE_STATUS_CHANGED: Self = Self(0xC184);
    pub const CANON_EOS_STORAGE_INFO_CHANGED: Self = Self(0xC185);
    pub const CANON_EOS_REQUEST_OBJECT_TRANSFER: Self = Self(0xC186);
    pub const CANON_EOS_OBJECT_INFO_CHANGED_EX: Self = Self(0xC187);
    pub const CANON_EOS_OBJECT_CONTENT_CHANGED: Self = Self(0xC188);
    pub const CANON_EOS_PROP_VALUE_CHANGED: Self = Self(0xC189);
    pub const CANON_EOS_AVAIL_LIST_CHANGED: Self = Self(0xC18A);
    pub const CANON_EOS_CAMERA_STATUS_CHANGED: Self = Self(0xC18B);
    pub const CANON_EOS_WILL_SOON_SHUTDOWN: Self = Self(0xC18D);
    pub const CANON_EOS_SHUTDOWN_TIMER_UPDATED: Self = Self(0xC18E);
    pub const CANON_EOS_REQUEST_CANCEL_TRANSFER: Self = Self(0xC18F);
    pub const CANON_EOS_REQUEST_OBJECT_TRANSFER_DT: Self = Self(0xC190);
    pub const CANON_EOS_REQUEST_CANCEL_TRANSFER_DT: Self = Self(0xC191);
    pub const CANON_EOS_STORE_ADDED: Self = Self(0xC192);
    pub const CANON_EOS_STORE_REMOVED: Self = Self(0xC193);
    pub const CANON_EOS_BULB_EXPOSURE_TIME: Self = Self(0xC194);
    pub const CANON_EOS_RECORDING_TIME: Self = Self(0xC195);
    pub const CANON_EOS_AF_RESULT: Self = Self(0xC1A3);

    /// Debug name from go-mtpfs `EC_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x4000 => "Undefined",
            0x4001 => "CancelTransaction",
            0x4002 => "ObjectAdded",
            0x4003 => "ObjectRemoved",
            0x4004 => "StoreAdded",
            0x4005 => "StoreRemoved",
            0x4006 => "DevicePropChanged",
            0x4007 => "ObjectInfoChanged",
            0x4008 => "DeviceInfoChanged",
            0x4009 => "RequestObjectTransfer",
            0x400A => "StoreFull",
            0x400B => "DeviceReset",
            0x400C => "StorageInfoChanged",
            0x400D => "CaptureComplete",
            0x400E => "UnreportedStatus",
            0xC101 => "Nikon_ObjectAddedInSDRAM",
            0xC102 => "Nikon_CaptureCompleteRecInSdram",
            0xC103 => "Nikon_AdvancedTransfer",
            0xC104 => "Nikon_PreviewImageAdded",
            0xC801 => "MTP_ObjectPropChanged",
            0xC802 => "MTP_ObjectPropDescChanged",
            0xC803 => "MTP_ObjectReferencesChanged",
            _ => return None,
        })
    }
}

impl fmt::Display for EventCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// object format code (`OFC_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ObjectFormat(pub u16);

impl ObjectFormat {
    pub const UNDEFINED: Self = Self(0x3000);
    pub const ASSOCIATION: Self = Self(0x3001);
    pub const SCRIPT: Self = Self(0x3002);
    pub const EXECUTABLE: Self = Self(0x3003);
    pub const TEXT: Self = Self(0x3004);
    pub const HTML: Self = Self(0x3005);
    pub const DPOF: Self = Self(0x3006);
    pub const AIFF: Self = Self(0x3007);
    pub const WAV: Self = Self(0x3008);
    pub const MP3: Self = Self(0x3009);
    pub const AVI: Self = Self(0x300A);
    pub const MPEG: Self = Self(0x300B);
    pub const ASF: Self = Self(0x300C);
    pub const DEFINED: Self = Self(0x3800);
    pub const EXIF_JPEG: Self = Self(0x3801);
    pub const TIFF_EP: Self = Self(0x3802);
    pub const FLASH_PIX: Self = Self(0x3803);
    pub const BMP: Self = Self(0x3804);
    pub const CIFF: Self = Self(0x3805);
    pub const UNDEFINED_0X3806: Self = Self(0x3806);
    pub const GIF: Self = Self(0x3807);
    pub const JFIF: Self = Self(0x3808);
    pub const PCD: Self = Self(0x3809);
    pub const PICT: Self = Self(0x380A);
    pub const PNG: Self = Self(0x380B);
    pub const UNDEFINED_0X380C: Self = Self(0x380C);
    pub const TIFF: Self = Self(0x380D);
    pub const TIFF_IT: Self = Self(0x380E);
    pub const JP2: Self = Self(0x380F);
    pub const JPX: Self = Self(0x3810);
    pub const DNG: Self = Self(0x3811);
    pub const EK_M3U: Self = Self(0xB002);
    pub const CANON_CRW: Self = Self(0xB101);
    pub const CANON_CRW3: Self = Self(0xB103);
    pub const CANON_MOV: Self = Self(0xB104);
    pub const CANON_CHDK_CRW: Self = Self(0xB1FF);
    pub const MTP_MEDIA_CARD: Self = Self(0xB211);
    pub const MTP_MEDIA_CARD_GROUP: Self = Self(0xB212);
    pub const MTP_ENCOUNTER: Self = Self(0xB213);
    pub const MTP_ENCOUNTER_BOX: Self = Self(0xB214);
    pub const MTP_M4A: Self = Self(0xB215);
    pub const MTP_FIRMWARE: Self = Self(0xB802);
    pub const MTP_WINDOWS_IMAGE_FORMAT: Self = Self(0xB881);
    pub const MTP_UNDEFINED_AUDIO: Self = Self(0xB900);
    pub const MTP_WMA: Self = Self(0xB901);
    pub const MTP_OGG: Self = Self(0xB902);
    pub const MTP_AAC: Self = Self(0xB903);
    pub const MTP_AUDIBLE_CODEC: Self = Self(0xB904);
    pub const MTP_FLAC: Self = Self(0xB906);
    pub const MTP_SAMSUNG_PLAYLIST: Self = Self(0xB909);
    pub const MTP_UNDEFINED_VIDEO: Self = Self(0xB980);
    pub const MTP_WMV: Self = Self(0xB981);
    pub const MTP_MP4: Self = Self(0xB982);
    pub const MTP_MP2: Self = Self(0xB983);
    pub const MTP_3GP: Self = Self(0xB984);
    pub const MTP_UNDEFINED_COLLECTION: Self = Self(0xBA00);
    pub const MTP_ABSTRACT_MULTIMEDIA_ALBUM: Self = Self(0xBA01);
    pub const MTP_ABSTRACT_IMAGE_ALBUM: Self = Self(0xBA02);
    pub const MTP_ABSTRACT_AUDIO_ALBUM: Self = Self(0xBA03);
    pub const MTP_ABSTRACT_VIDEO_ALBUM: Self = Self(0xBA04);
    pub const MTP_ABSTRACT_AUDIO_VIDEO_PLAYLIST: Self = Self(0xBA05);
    pub const MTP_ABSTRACT_CONTACT_GROUP: Self = Self(0xBA06);
    pub const MTP_ABSTRACT_MESSAGE_FOLDER: Self = Self(0xBA07);
    pub const MTP_ABSTRACT_CHAPTERED_PRODUCTION: Self = Self(0xBA08);
    pub const MTP_ABSTRACT_AUDIO_PLAYLIST: Self = Self(0xBA09);
    pub const MTP_ABSTRACT_VIDEO_PLAYLIST: Self = Self(0xBA0A);
    pub const MTP_ABSTRACT_MEDIACAST: Self = Self(0xBA0B);
    pub const MTP_WPL_PLAYLIST: Self = Self(0xBA10);
    pub const MTP_M3U_PLAYLIST: Self = Self(0xBA11);
    pub const MTP_MPL_PLAYLIST: Self = Self(0xBA12);
    pub const MTP_ASX_PLAYLIST: Self = Self(0xBA13);
    pub const MTP_PLS_PLAYLIST: Self = Self(0xBA14);
    pub const MTP_UNDEFINED_DOCUMENT: Self = Self(0xBA80);
    pub const MTP_ABSTRACT_DOCUMENT: Self = Self(0xBA81);
    pub const MTP_XML_DOCUMENT: Self = Self(0xBA82);
    pub const MTP_MS_WORD_DOCUMENT: Self = Self(0xBA83);
    pub const MTP_MHT_COMPILED_HTML_DOCUMENT: Self = Self(0xBA84);
    pub const MTP_MS_EXCEL_SPREADSHEET_XLS: Self = Self(0xBA85);
    pub const MTP_MS_POWERPOINT_PRESENTATION_PPT: Self = Self(0xBA86);
    pub const MTP_UNDEFINED_MESSAGE: Self = Self(0xBB00);
    pub const MTP_ABSTRACT_MESSAGE: Self = Self(0xBB01);
    pub const MTP_UNDEFINED_CONTACT: Self = Self(0xBB80);
    pub const MTP_ABSTRACT_CONTACT: Self = Self(0xBB81);
    pub const MTP_V_CARD2: Self = Self(0xBB82);
    pub const MTP_V_CARD3: Self = Self(0xBB83);
    pub const MTP_UNDEFINED_CALENDAR_ITEM: Self = Self(0xBE00);
    pub const MTP_ABSTRACT_CALENDAR_ITEM: Self = Self(0xBE01);
    pub const MTP_V_CALENDAR1: Self = Self(0xBE02);
    pub const MTP_V_CALENDAR2: Self = Self(0xBE03);
    pub const MTP_UNDEFINED_WINDOWS_EXECUTABLE: Self = Self(0xBE80);
    pub const MTP_MEDIA_CAST: Self = Self(0xBE81);
    pub const MTP_SECTION: Self = Self(0xBE82);

    /// Debug name from go-mtpfs `OFC_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x3000 => "Undefined",
            0x3001 => "Association",
            0x3002 => "Script",
            0x3003 => "Executable",
            0x3004 => "Text",
            0x3005 => "HTML",
            0x3006 => "DPOF",
            0x3007 => "AIFF",
            0x3008 => "WAV",
            0x3009 => "MP3",
            0x300A => "AVI",
            0x300B => "MPEG",
            0x300C => "ASF",
            0x3800 => "Defined",
            0x3801 => "EXIF_JPEG",
            0x3802 => "TIFF_EP",
            0x3803 => "FlashPix",
            0x3804 => "BMP",
            0x3805 => "CIFF",
            0x3806 => "Undefined_0x3806",
            0x3807 => "GIF",
            0x3808 => "JFIF",
            0x3809 => "PCD",
            0x380A => "PICT",
            0x380B => "PNG",
            0x380C => "Undefined_0x380C",
            0x380D => "TIFF",
            0x380E => "TIFF_IT",
            0x380F => "JP2",
            0x3810 => "JPX",
            0x3811 => "DNG",
            0xB211 => "MTP_MediaCard",
            0xB212 => "MTP_MediaCardGroup",
            0xB213 => "MTP_Encounter",
            0xB214 => "MTP_EncounterBox",
            0xB215 => "MTP_M4A",
            0xB802 => "MTP_Firmware",
            0xB881 => "MTP_WindowsImageFormat",
            0xB900 => "MTP_UndefinedAudio",
            0xB901 => "MTP_WMA",
            0xB902 => "MTP_OGG",
            0xB903 => "MTP_AAC",
            0xB904 => "MTP_AudibleCodec",
            0xB906 => "MTP_FLAC",
            0xB909 => "MTP_SamsungPlaylist",
            0xB980 => "MTP_UndefinedVideo",
            0xB981 => "MTP_WMV",
            0xB982 => "MTP_MP4",
            0xB983 => "MTP_MP2",
            0xB984 => "MTP_3GP",
            0xBA00 => "MTP_UndefinedCollection",
            0xBA01 => "MTP_AbstractMultimediaAlbum",
            0xBA02 => "MTP_AbstractImageAlbum",
            0xBA03 => "MTP_AbstractAudioAlbum",
            0xBA04 => "MTP_AbstractVideoAlbum",
            0xBA05 => "MTP_AbstractAudioVideoPlaylist",
            0xBA06 => "MTP_AbstractContactGroup",
            0xBA07 => "MTP_AbstractMessageFolder",
            0xBA08 => "MTP_AbstractChapteredProduction",
            0xBA09 => "MTP_AbstractAudioPlaylist",
            0xBA0A => "MTP_AbstractVideoPlaylist",
            0xBA0B => "MTP_AbstractMediacast",
            0xBA10 => "MTP_WPLPlaylist",
            0xBA11 => "MTP_M3UPlaylist",
            0xBA12 => "MTP_MPLPlaylist",
            0xBA13 => "MTP_ASXPlaylist",
            0xBA14 => "MTP_PLSPlaylist",
            0xBA80 => "MTP_UndefinedDocument",
            0xBA81 => "MTP_AbstractDocument",
            0xBA82 => "MTP_XMLDocument",
            0xBA83 => "MTP_MSWordDocument",
            0xBA84 => "MTP_MHTCompiledHTMLDocument",
            0xBA85 => "MTP_MSExcelSpreadsheetXLS",
            0xBA86 => "MTP_MSPowerpointPresentationPPT",
            0xBB00 => "MTP_UndefinedMessage",
            0xBB01 => "MTP_AbstractMessage",
            0xBB80 => "MTP_UndefinedContact",
            0xBB81 => "MTP_AbstractContact",
            0xBB82 => "MTP_vCard2",
            0xBB83 => "MTP_vCard3",
            0xBE00 => "MTP_UndefinedCalendarItem",
            0xBE01 => "MTP_AbstractCalendarItem",
            0xBE02 => "MTP_vCalendar1",
            0xBE03 => "MTP_vCalendar2",
            0xBE80 => "MTP_UndefinedWindowsExecutable",
            0xBE81 => "MTP_MediaCast",
            0xBE82 => "MTP_Section",
            _ => return None,
        })
    }
}

impl fmt::Display for ObjectFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// device property code (`DPC_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct DevicePropCode(pub u16);

impl DevicePropCode {
    pub const UNDEFINED: Self = Self(0x5000);
    pub const BATTERY_LEVEL: Self = Self(0x5001);
    pub const FUNCTIONAL_MODE: Self = Self(0x5002);
    pub const IMAGE_SIZE: Self = Self(0x5003);
    pub const COMPRESSION_SETTING: Self = Self(0x5004);
    pub const WHITE_BALANCE: Self = Self(0x5005);
    pub const RGB_GAIN: Self = Self(0x5006);
    pub const F_NUMBER: Self = Self(0x5007);
    pub const FOCAL_LENGTH: Self = Self(0x5008);
    pub const FOCUS_DISTANCE: Self = Self(0x5009);
    pub const FOCUS_MODE: Self = Self(0x500A);
    pub const EXPOSURE_METERING_MODE: Self = Self(0x500B);
    pub const FLASH_MODE: Self = Self(0x500C);
    pub const EXPOSURE_TIME: Self = Self(0x500D);
    pub const EXPOSURE_PROGRAM_MODE: Self = Self(0x500E);
    pub const EXPOSURE_INDEX: Self = Self(0x500F);
    pub const EXPOSURE_BIAS_COMPENSATION: Self = Self(0x5010);
    pub const DATE_TIME: Self = Self(0x5011);
    pub const CAPTURE_DELAY: Self = Self(0x5012);
    pub const STILL_CAPTURE_MODE: Self = Self(0x5013);
    pub const CONTRAST: Self = Self(0x5014);
    pub const SHARPNESS: Self = Self(0x5015);
    pub const DIGITAL_ZOOM: Self = Self(0x5016);
    pub const EFFECT_MODE: Self = Self(0x5017);
    pub const BURST_NUMBER: Self = Self(0x5018);
    pub const BURST_INTERVAL: Self = Self(0x5019);
    pub const TIMELAPSE_NUMBER: Self = Self(0x501A);
    pub const TIMELAPSE_INTERVAL: Self = Self(0x501B);
    pub const FOCUS_METERING_MODE: Self = Self(0x501C);
    pub const UPLOAD_URL: Self = Self(0x501D);
    pub const ARTIST: Self = Self(0x501E);
    pub const COPYRIGHT_INFO: Self = Self(0x501F);
    pub const SUPPORTED_STREAMS: Self = Self(0x5020);
    pub const ENABLED_STREAMS: Self = Self(0x5021);
    pub const VIDEO_FORMAT: Self = Self(0x5022);
    pub const VIDEO_RESOLUTION: Self = Self(0x5023);
    pub const VIDEO_QUALITY: Self = Self(0x5024);
    pub const VIDEO_FRAME_RATE: Self = Self(0x5025);
    pub const VIDEO_CONTRAST: Self = Self(0x5026);
    pub const VIDEO_BRIGHTNESS: Self = Self(0x5027);
    pub const AUDIO_FORMAT: Self = Self(0x5028);
    pub const AUDIO_BITRATE: Self = Self(0x5029);
    pub const AUDIO_SAMPLING_RATE: Self = Self(0x502A);
    pub const AUDIO_BIT_PER_SAMPLE: Self = Self(0x502B);
    pub const AUDIO_VOLUME: Self = Self(0x502C);
    pub const EXTENSION: Self = Self(0xD000);
    pub const CANON_BEEP_MODE: Self = Self(0xD001);
    pub const EK_COLOR_TEMPERATURE: Self = Self(0xD001);
    pub const CANON_BATTERY_KIND: Self = Self(0xD002);
    pub const EK_DATE_TIME_STAMP_FORMAT: Self = Self(0xD002);
    pub const CANON_BATTERY_STATUS: Self = Self(0xD003);
    pub const EK_BEEP_MODE: Self = Self(0xD003);
    pub const CANON_UI_LOCK_TYPE: Self = Self(0xD004);
    pub const CASIO_UNKNOWN_1: Self = Self(0xD004);
    pub const EK_VIDEO_OUT: Self = Self(0xD004);
    pub const CANON_CAMERA_MODE: Self = Self(0xD005);
    pub const CASIO_UNKNOWN_2: Self = Self(0xD005);
    pub const EK_POWER_SAVING: Self = Self(0xD005);
    pub const CANON_IMAGE_QUALITY: Self = Self(0xD006);
    pub const EK_UI_LANGUAGE: Self = Self(0xD006);
    pub const CANON_FULL_VIEW_FILE_FORMAT: Self = Self(0xD007);
    pub const CASIO_UNKNOWN_3: Self = Self(0xD007);
    pub const CANON_IMAGE_SIZE: Self = Self(0xD008);
    pub const CASIO_RECORD_LIGHT: Self = Self(0xD008);
    pub const CANON_SELF_TIME: Self = Self(0xD009);
    pub const CASIO_UNKNOWN_4: Self = Self(0xD009);
    pub const CANON_FLASH_MODE: Self = Self(0xD00A);
    pub const CASIO_UNKNOWN_5: Self = Self(0xD00A);
    pub const CANON_BEEP: Self = Self(0xD00B);
    pub const CASIO_MOVIE_MODE: Self = Self(0xD00B);
    pub const CANON_SHOOTING_MODE: Self = Self(0xD00C);
    pub const CASIO_HD_SETTING: Self = Self(0xD00C);
    pub const CANON_IMAGE_MODE: Self = Self(0xD00D);
    pub const CASIO_HS_SETTING: Self = Self(0xD00D);
    pub const CANON_DRIVE_MODE: Self = Self(0xD00E);
    pub const CANON_E_ZOOM: Self = Self(0xD00F);
    pub const CASIO_CS_HIGH_SPEED: Self = Self(0xD00F);
    pub const CANON_METERING_MODE: Self = Self(0xD010);
    pub const CASIO_CS_UPPER_LIMIT: Self = Self(0xD010);
    pub const NIKON_SHOOTING_BANK: Self = Self(0xD010);
    pub const CANON_AF_DISTANCE: Self = Self(0xD011);
    pub const CASIO_CS_SHOT: Self = Self(0xD011);
    pub const NIKON_SHOOTING_BANK_NAME_A: Self = Self(0xD011);
    pub const CANON_FOCUSING_POINT: Self = Self(0xD012);
    pub const CASIO_UNKNOWN_6: Self = Self(0xD012);
    pub const NIKON_SHOOTING_BANK_NAME_B: Self = Self(0xD012);
    pub const CANON_WHITE_BALANCE: Self = Self(0xD013);
    pub const CASIO_UNKNOWN_7: Self = Self(0xD013);
    pub const NIKON_SHOOTING_BANK_NAME_C: Self = Self(0xD013);
    pub const CANON_SLOW_SHUTTER_SETTING: Self = Self(0xD014);
    pub const NIKON_SHOOTING_BANK_NAME_D: Self = Self(0xD014);
    pub const CANON_AF_MODE: Self = Self(0xD015);
    pub const CASIO_UNKNOWN_8: Self = Self(0xD015);
    pub const NIKON_RESET_BANK0: Self = Self(0xD015);
    pub const CANON_IMAGE_STABILIZATION: Self = Self(0xD016);
    pub const NIKON_RAW_COMPRESSION: Self = Self(0xD016);
    pub const CANON_CONTRAST: Self = Self(0xD017);
    pub const CASIO_UNKNOWN_9: Self = Self(0xD017);
    pub const FUJI_COLOR_TEMPERATURE: Self = Self(0xD017);
    pub const NIKON_WHITE_BALANCE_AUTO_BIAS: Self = Self(0xD017);
    pub const CANON_COLOR_GAIN: Self = Self(0xD018);
    pub const CASIO_UNKNOWN_10: Self = Self(0xD018);
    pub const FUJI_QUALITY: Self = Self(0xD018);
    pub const NIKON_WHITE_BALANCE_TUNGSTEN_BIAS: Self = Self(0xD018);
    pub const CANON_SHARPNESS: Self = Self(0xD019);
    pub const CASIO_UNKNOWN_11: Self = Self(0xD019);
    pub const NIKON_WHITE_BALANCE_FLUORESCENT_BIAS: Self = Self(0xD019);
    pub const CANON_SENSITIVITY: Self = Self(0xD01A);
    pub const CASIO_UNKNOWN_12: Self = Self(0xD01A);
    pub const NIKON_WHITE_BALANCE_DAYLIGHT_BIAS: Self = Self(0xD01A);
    pub const CANON_PARAMETER_SET: Self = Self(0xD01B);
    pub const CASIO_UNKNOWN_13: Self = Self(0xD01B);
    pub const NIKON_WHITE_BALANCE_FLASH_BIAS: Self = Self(0xD01B);
    pub const CANON_ISO_SPEED: Self = Self(0xD01C);
    pub const CASIO_UNKNOWN_14: Self = Self(0xD01C);
    pub const NIKON_WHITE_BALANCE_CLOUDY_BIAS: Self = Self(0xD01C);
    pub const CANON_APERTURE: Self = Self(0xD01D);
    pub const CASIO_UNKNOWN_15: Self = Self(0xD01D);
    pub const NIKON_WHITE_BALANCE_SHADE_BIAS: Self = Self(0xD01D);
    pub const CANON_SHUTTER_SPEED: Self = Self(0xD01E);
    pub const NIKON_WHITE_BALANCE_COLOR_TEMPERATURE: Self = Self(0xD01E);
    pub const CANON_EXP_COMPENSATION: Self = Self(0xD01F);
    pub const NIKON_WHITE_BALANCE_PRESET_NO: Self = Self(0xD01F);
    pub const CANON_FLASH_COMPENSATION: Self = Self(0xD020);
    pub const CASIO_UNKNOWN_16: Self = Self(0xD020);
    pub const NIKON_WHITE_BALANCE_PRESET_NAME0: Self = Self(0xD020);
    pub const CANON_AEB_EXPOSURE_COMPENSATION: Self = Self(0xD021);
    pub const NIKON_WHITE_BALANCE_PRESET_NAME1: Self = Self(0xD021);
    pub const NIKON_WHITE_BALANCE_PRESET_NAME2: Self = Self(0xD022);
    pub const CANON_AV_OPEN: Self = Self(0xD023);
    pub const NIKON_WHITE_BALANCE_PRESET_NAME3: Self = Self(0xD023);
    pub const CANON_AV_MAX: Self = Self(0xD024);
    pub const NIKON_WHITE_BALANCE_PRESET_NAME4: Self = Self(0xD024);
    pub const CANON_FOCAL_LENGTH: Self = Self(0xD025);
    pub const NIKON_WHITE_BALANCE_PRESET_VAL0: Self = Self(0xD025);
    pub const CANON_FOCAL_LENGTH_TELE: Self = Self(0xD026);
    pub const NIKON_WHITE_BALANCE_PRESET_VAL1: Self = Self(0xD026);
    pub const CANON_FOCAL_LENGTH_WIDE: Self = Self(0xD027);
    pub const NIKON_WHITE_BALANCE_PRESET_VAL2: Self = Self(0xD027);
    pub const CANON_FOCAL_LENGTH_DENOMINATOR: Self = Self(0xD028);
    pub const NIKON_WHITE_BALANCE_PRESET_VAL3: Self = Self(0xD028);
    pub const CANON_CAPTURE_TRANSFER_MODE: Self = Self(0xD029);
    pub const NIKON_WHITE_BALANCE_PRESET_VAL4: Self = Self(0xD029);
    pub const CANON_ZOOM: Self = Self(0xD02A);
    pub const NIKON_IMAGE_SHARPENING: Self = Self(0xD02A);
    pub const CANON_NAME_PREFIX: Self = Self(0xD02B);
    pub const NIKON_TONE_COMPENSATION: Self = Self(0xD02B);
    pub const CANON_SIZE_QUALITY_MODE: Self = Self(0xD02C);
    pub const NIKON_COLOR_MODEL: Self = Self(0xD02C);
    pub const CANON_SUPPORTED_THUMB_SIZE: Self = Self(0xD02D);
    pub const NIKON_HUE_ADJUSTMENT: Self = Self(0xD02D);
    pub const CANON_SIZE_OF_OUTPUT_DATA_FROM_CAMERA: Self = Self(0xD02E);
    pub const CANON_SIZE_OF_INPUT_DATA_TO_CAMERA: Self = Self(0xD02F);
    pub const CANON_REMOTE_API_VERSION: Self = Self(0xD030);
    pub const CASIO_UNKNOWN_17: Self = Self(0xD030);
    pub const NIKON_SHOOTING_MODE: Self = Self(0xD030);
    pub const CANON_FIRMWARE_VERSION: Self = Self(0xD031);
    pub const NIKON_JPEG_COMPRESSION_POLICY: Self = Self(0xD031);
    pub const CANON_CAMERA_MODEL: Self = Self(0xD032);
    pub const NIKON_COLOR_SPACE: Self = Self(0xD032);
    pub const CANON_CAMERA_OWNER: Self = Self(0xD033);
    pub const NIKON_AUTO_DX_CROP: Self = Self(0xD033);
    pub const CANON_UNIX_TIME: Self = Self(0xD034);
    pub const CANON_CAMERA_BODY_ID: Self = Self(0xD035);
    pub const CANON_CAMERA_OUTPUT: Self = Self(0xD036);
    pub const NIKON_VIDEO_MODE: Self = Self(0xD036);
    pub const CANON_DISP_AV: Self = Self(0xD037);
    pub const NIKON_EFFECT_MODE: Self = Self(0xD037);
    pub const CANON_AV_OPEN_APEX: Self = Self(0xD038);
    pub const CANON_D_ZOOM_MAGNIFICATION: Self = Self(0xD039);
    pub const CANON_ML_SPOT_POS: Self = Self(0xD03A);
    pub const CANON_DISP_AV_MAX: Self = Self(0xD03B);
    pub const CANON_AV_MAX_APEX: Self = Self(0xD03C);
    pub const CANON_E_ZOOM_START_POSITION: Self = Self(0xD03D);
    pub const CANON_FOCAL_LENGTH_OF_TELE: Self = Self(0xD03E);
    pub const CANON_E_ZOOM_SIZE_OF_TELE: Self = Self(0xD03F);
    pub const CANON_PHOTO_EFFECT: Self = Self(0xD040);
    pub const NIKON_CSM_MENU_BANK_SELECT: Self = Self(0xD040);
    pub const CANON_ASSIST_LIGHT: Self = Self(0xD041);
    pub const NIKON_MENU_BANK_NAME_A: Self = Self(0xD041);
    pub const CANON_FLASH_QUANTITY_COUNT: Self = Self(0xD042);
    pub const NIKON_MENU_BANK_NAME_B: Self = Self(0xD042);
    pub const CANON_ROTATION_ANGLE: Self = Self(0xD043);
    pub const NIKON_MENU_BANK_NAME_C: Self = Self(0xD043);
    pub const CANON_ROTATION_SCENE: Self = Self(0xD044);
    pub const NIKON_MENU_BANK_NAME_D: Self = Self(0xD044);
    pub const CANON_EVENT_EMULATE_MODE: Self = Self(0xD045);
    pub const NIKON_RESET_BANK: Self = Self(0xD045);
    pub const CANON_DPOF_VERSION: Self = Self(0xD046);
    pub const CANON_TYPE_OF_SUPPORTED_SLIDE_SHOW: Self = Self(0xD047);
    pub const CANON_AVERAGE_FILESIZES: Self = Self(0xD048);
    pub const NIKON_A1AFC_MODE_PRIORITY: Self = Self(0xD048);
    pub const CANON_MODEL_ID: Self = Self(0xD049);
    pub const NIKON_A2AFS_MODE_PRIORITY: Self = Self(0xD049);
    pub const NIKON_A3GROUP_DYNAMIC_AF: Self = Self(0xD04A);
    pub const NIKON_A4AF_ACTIVATION: Self = Self(0xD04B);
    pub const NIKON_FOCUS_AREA_ILLUM_MANUAL_FOCUS: Self = Self(0xD04C);
    pub const NIKON_FOCUS_AREA_ILLUM_CONTINUOUS: Self = Self(0xD04D);
    pub const NIKON_FOCUS_AREA_ILLUM_WHEN_SELECTED: Self = Self(0xD04E);
    pub const NIKON_VERTICAL_AFON: Self = Self(0xD050);
    pub const NIKON_AF_LOCK_ON: Self = Self(0xD051);
    pub const NIKON_FOCUS_AREA_ZONE: Self = Self(0xD052);
    pub const NIKON_ENABLE_COPYRIGHT: Self = Self(0xD053);
    pub const NIKON_ISO_AUTO: Self = Self(0xD054);
    pub const NIKON_EVISO_STEP: Self = Self(0xD055);
    pub const NIKON_EV_STEP_EXPOSURE_COMP: Self = Self(0xD057);
    pub const NIKON_EXPOSURE_COMPENSATION: Self = Self(0xD058);
    pub const NIKON_CENTER_WEIGHT_AREA: Self = Self(0xD059);
    pub const NIKON_EXPOSURE_BASE_MATRIX: Self = Self(0xD05A);
    pub const NIKON_EXPOSURE_BASE_CENTER: Self = Self(0xD05B);
    pub const NIKON_EXPOSURE_BASE_SPOT: Self = Self(0xD05C);
    pub const NIKON_LIVE_VIEW_AF_AREA: Self = Self(0xD05D);
    pub const NIKON_AE_LOCK_MODE: Self = Self(0xD05E);
    pub const NIKON_AELAFL_MODE: Self = Self(0xD05F);
    pub const NIKON_LIVE_VIEW_AF_FOCUS: Self = Self(0xD061);
    pub const NIKON_METER_OFF: Self = Self(0xD062);
    pub const NIKON_SELF_TIMER: Self = Self(0xD063);
    pub const NIKON_MONITOR_OFF: Self = Self(0xD064);
    pub const NIKON_IMG_CONF_TIME: Self = Self(0xD065);
    pub const NIKON_AUTO_OFF_TIMERS: Self = Self(0xD066);
    pub const NIKON_ANGLE_LEVEL: Self = Self(0xD067);
    pub const NIKON_D2MAXIMUM_SHOTS: Self = Self(0xD069);
    pub const NIKON_EXPOSURE_DELAY_MODE: Self = Self(0xD06A);
    pub const NIKON_LONG_EXPOSURE_NOISE_REDUCTION: Self = Self(0xD06B);
    pub const NIKON_FILE_NUMBER_SEQUENCE: Self = Self(0xD06C);
    pub const NIKON_CONTROL_PANEL_FINDER_REAR_CONTROL: Self = Self(0xD06D);
    pub const NIKON_CONTROL_PANEL_FINDER_VIEWFINDER: Self = Self(0xD06E);
    pub const NIKON_D7ILLUMINATION: Self = Self(0xD06F);
    pub const NIKON_NR_HIGH_ISO: Self = Self(0xD070);
    pub const NIKON_SHSET_CH_GUID_DISP: Self = Self(0xD071);
    pub const NIKON_ARTIST_NAME: Self = Self(0xD072);
    pub const NIKON_COPYRIGHT_INFO: Self = Self(0xD073);
    pub const NIKON_FLASH_SYNC_SPEED: Self = Self(0xD074);
    pub const NIKON_E3AA_FLASH_MODE: Self = Self(0xD076);
    pub const NIKON_E4MODELING_FLASH: Self = Self(0xD077);
    pub const NIKON_BRACKET_ORDER: Self = Self(0xD07A);
    pub const NIKON_BRACKETING_SET: Self = Self(0xD07C);
    pub const CASIO_UNKNOWN_18: Self = Self(0xD080);
    pub const NIKON_F1CENTER_BUTTON_SHOOTING_MODE: Self = Self(0xD080);
    pub const NIKON_CENTER_BUTTON_PLAYBACK_MODE: Self = Self(0xD081);
    pub const NIKON_F2MULTISELECTOR: Self = Self(0xD082);
    pub const NIKON_CENTER_BUTTON_ZOOM_RATIO: Self = Self(0xD08B);
    pub const NIKON_FUNCTION_BUTTON2: Self = Self(0xD08C);
    pub const NIKON_AF_AREA_POINT: Self = Self(0xD08D);
    pub const NIKON_NORMAL_AF_ON: Self = Self(0xD08E);
    pub const NIKON_CLEAN_IMAGE_SENSOR: Self = Self(0xD08F);
    pub const NIKON_IMAGE_COMMENT_STRING: Self = Self(0xD090);
    pub const NIKON_IMAGE_COMMENT_ENABLE: Self = Self(0xD091);
    pub const NIKON_IMAGE_ROTATION: Self = Self(0xD092);
    pub const NIKON_MANUAL_SET_LENS_NO: Self = Self(0xD093);
    pub const NIKON_MOV_SCREEN_SIZE: Self = Self(0xD0A0);
    pub const NIKON_MOV_VOICE: Self = Self(0xD0A1);
    pub const NIKON_MOV_MICROPHONE: Self = Self(0xD0A2);
    pub const NIKON_BRACKETING: Self = Self(0xD0C0);
    pub const NIKON_AUTO_EXPOSURE_BRACKET_STEP: Self = Self(0xD0C1);
    pub const NIKON_AUTO_EXPOSURE_BRACKET_PROGRAM: Self = Self(0xD0C2);
    pub const NIKON_AUTO_EXPOSURE_BRACKET_COUNT: Self = Self(0xD0C3);
    pub const NIKON_WHITE_BALANCE_BRACKET_STEP: Self = Self(0xD0C4);
    pub const NIKON_WHITE_BALANCE_BRACKET_PROGRAM: Self = Self(0xD0C5);
    pub const NIKON_LENS_ID: Self = Self(0xD0E0);
    pub const NIKON_LENS_SORT: Self = Self(0xD0E1);
    pub const NIKON_LENS_TYPE: Self = Self(0xD0E2);
    pub const NIKON_FOCAL_LENGTH_MIN: Self = Self(0xD0E3);
    pub const NIKON_FOCAL_LENGTH_MAX: Self = Self(0xD0E4);
    pub const NIKON_MAX_AP_AT_MIN_FOCAL_LENGTH: Self = Self(0xD0E5);
    pub const NIKON_MAX_AP_AT_MAX_FOCAL_LENGTH: Self = Self(0xD0E6);
    pub const NIKON_FINDER_ISO_DISP: Self = Self(0xD0F0);
    pub const NIKON_AUTO_OFF_PHOTO: Self = Self(0xD0F2);
    pub const NIKON_AUTO_OFF_MENU: Self = Self(0xD0F3);
    pub const NIKON_AUTO_OFF_INFO: Self = Self(0xD0F4);
    pub const NIKON_SELF_TIMER_SHOOT_NUM: Self = Self(0xD0F5);
    pub const NIKON_VIGNETTE_CTRL: Self = Self(0xD0F7);
    pub const NIKON_AUTO_DISTORTION_CONTROL: Self = Self(0xD0F8);
    pub const NIKON_SCENE_MODE: Self = Self(0xD0F9);
    pub const CANON_EOS_APERTURE: Self = Self(0xD101);
    pub const MTP_SECURE_TIME: Self = Self(0xD101);
    pub const NIKON_AC_POWER: Self = Self(0xD101);
    pub const CANON_EOS_SHUTTER_SPEED: Self = Self(0xD102);
    pub const MTP_DEVICE_CERTIFICATE: Self = Self(0xD102);
    pub const NIKON_WARNING_STATUS: Self = Self(0xD102);
    pub const OLYMPUS_RESOLUTION_MODE: Self = Self(0xD102);
    pub const CANON_EOS_ISO_SPEED: Self = Self(0xD103);
    pub const MTP_REVOCATION_INFO: Self = Self(0xD103);
    pub const OLYMPUS_FOCUS_PRIORITY: Self = Self(0xD103);
    pub const CANON_EOS_EXP_COMPENSATION: Self = Self(0xD104);
    pub const NIKON_AF_LOCK_STATUS: Self = Self(0xD104);
    pub const OLYMPUS_DRIVE_MODE: Self = Self(0xD104);
    pub const CANON_EOS_AUTO_EXPOSURE_MODE: Self = Self(0xD105);
    pub const NIKON_AE_LOCK_STATUS: Self = Self(0xD105);
    pub const OLYMPUS_DATE_TIME_FORMAT: Self = Self(0xD105);
    pub const CANON_EOS_DRIVE_MODE: Self = Self(0xD106);
    pub const NIKON_FV_LOCK_STATUS: Self = Self(0xD106);
    pub const OLYMPUS_EXPOSURE_BIAS_STEP: Self = Self(0xD106);
    pub const NIKON_AUTOFOCUS_LCD_TOP_MODE2: Self = Self(0xD107);
    pub const OLYMPUS_WB_MODE: Self = Self(0xD107);
    pub const CANON_EOS_FOCUS_MODE: Self = Self(0xD108);
    pub const NIKON_AUTOFOCUS_AREA: Self = Self(0xD108);
    pub const OLYMPUS_ONE_TOUCH_WB: Self = Self(0xD108);
    pub const CANON_EOS_WHITE_BALANCE: Self = Self(0xD109);
    pub const NIKON_FLEXIBLE_PROGRAM: Self = Self(0xD109);
    pub const OLYMPUS_MANUAL_WB: Self = Self(0xD109);
    pub const CANON_EOS_COLOR_TEMPERATURE: Self = Self(0xD10A);
    pub const OLYMPUS_MANUAL_WBRB_BIAS: Self = Self(0xD10A);
    pub const CANON_EOS_WHITE_BALANCE_ADJUST_A: Self = Self(0xD10B);
    pub const OLYMPUS_CUSTOM_WB: Self = Self(0xD10B);
    pub const CANON_EOS_WHITE_BALANCE_ADJUST_B: Self = Self(0xD10C);
    pub const NIKON_USB_SPEED: Self = Self(0xD10C);
    pub const OLYMPUS_CUSTOM_WB_VALUE: Self = Self(0xD10C);
    pub const CANON_EOS_WHITE_BALANCE_XA: Self = Self(0xD10D);
    pub const NIKON_CCD_NUMBER: Self = Self(0xD10D);
    pub const OLYMPUS_EXPOSURE_TIME_EX: Self = Self(0xD10D);
    pub const CANON_EOS_WHITE_BALANCE_XB: Self = Self(0xD10E);
    pub const NIKON_CAMERA_ORIENTATION: Self = Self(0xD10E);
    pub const OLYMPUS_BULB_MODE: Self = Self(0xD10E);
    pub const CANON_EOS_COLOR_SPACE: Self = Self(0xD10F);
    pub const NIKON_GROUP_PTN_TYPE: Self = Self(0xD10F);
    pub const OLYMPUS_ANTI_MIRROR_MODE: Self = Self(0xD10F);
    pub const CANON_EOS_PICTURE_STYLE: Self = Self(0xD110);
    pub const NIKON_F_NUMBER_LOCK: Self = Self(0xD110);
    pub const OLYMPUS_AE_BRACKETING_FRAME: Self = Self(0xD110);
    pub const CANON_EOS_BATTERY_POWER: Self = Self(0xD111);
    pub const OLYMPUS_AE_BRACKETING_STEP: Self = Self(0xD111);
    pub const CANON_EOS_BATTERY_SELECT: Self = Self(0xD112);
    pub const NIKON_TV_LOCK_SETTING: Self = Self(0xD112);
    pub const OLYMPUS_WB_BRACKETING_FRAME: Self = Self(0xD112);
    pub const CANON_EOS_CAMERA_TIME: Self = Self(0xD113);
    pub const NIKON_AV_LOCK_SETTING: Self = Self(0xD113);
    pub const OLYMPUS_WB_BRACKETING_RB_RANGE: Self = Self(0xD113);
    pub const NIKON_ILLUM_SETTING: Self = Self(0xD114);
    pub const OLYMPUS_WB_BRACKETING_GM_FRAME: Self = Self(0xD114);
    pub const CANON_EOS_OWNER: Self = Self(0xD115);
    pub const NIKON_FOCUS_POINT_BRIGHT: Self = Self(0xD115);
    pub const OLYMPUS_WB_BRACKETING_GM_RANGE: Self = Self(0xD115);
    pub const CANON_EOS_MODEL_ID: Self = Self(0xD116);
    pub const OLYMPUS_FL_BRACKETING_FRAME: Self = Self(0xD118);
    pub const CANON_EOS_PTP_EXTENSION_VERSION: Self = Self(0xD119);
    pub const OLYMPUS_FL_BRACKETING_STEP: Self = Self(0xD119);
    pub const CANON_EOS_DPOF_VERSION: Self = Self(0xD11A);
    pub const OLYMPUS_FLASH_BIAS_COMPENSATION: Self = Self(0xD11A);
    pub const CANON_EOS_AVAILABLE_SHOTS: Self = Self(0xD11B);
    pub const OLYMPUS_MANUAL_FOCUS_MODE: Self = Self(0xD11B);
    pub const CANON_EOS_CAPTURE_DESTINATION: Self = Self(0xD11C);
    pub const CANON_EOS_BRACKET_MODE: Self = Self(0xD11D);
    pub const OLYMPUS_RAW_SAVE_MODE: Self = Self(0xD11D);
    pub const CANON_EOS_CURRENT_STORAGE: Self = Self(0xD11E);
    pub const OLYMPUS_AUX_LIGHT_MODE: Self = Self(0xD11E);
    pub const CANON_EOS_CURRENT_FOLDER: Self = Self(0xD11F);
    pub const OLYMPUS_LENS_SINK_MODE: Self = Self(0xD11F);
    pub const NIKON_EXTERNAL_FLASH_ATTACHED: Self = Self(0xD120);
    pub const OLYMPUS_BEEP_STATUS: Self = Self(0xD120);
    pub const NIKON_EXTERNAL_FLASH_STATUS: Self = Self(0xD121);
    pub const NIKON_EXTERNAL_FLASH_SORT: Self = Self(0xD122);
    pub const OLYMPUS_COLOR_SPACE: Self = Self(0xD122);
    pub const NIKON_EXTERNAL_FLASH_MODE: Self = Self(0xD123);
    pub const OLYMPUS_COLOR_MATCHING: Self = Self(0xD123);
    pub const NIKON_EXTERNAL_FLASH_COMPENSATION: Self = Self(0xD124);
    pub const OLYMPUS_SATURATION: Self = Self(0xD124);
    pub const NIKON_NEW_EXTERNAL_FLASH_MODE: Self = Self(0xD125);
    pub const NIKON_FLASH_EXPOSURE_COMPENSATION: Self = Self(0xD126);
    pub const OLYMPUS_NOISE_REDUCTION_PATTERN: Self = Self(0xD126);
    pub const OLYMPUS_NOISE_REDUCTION_RANDOM: Self = Self(0xD127);
    pub const OLYMPUS_SHADING_MODE: Self = Self(0xD129);
    pub const OLYMPUS_ISO_BOOST_MODE: Self = Self(0xD12A);
    pub const OLYMPUS_EXPOSURE_INDEX_BIAS_STEP: Self = Self(0xD12B);
    pub const OLYMPUS_FILTER_EFFECT: Self = Self(0xD12C);
    pub const OLYMPUS_COLOR_TUNE: Self = Self(0xD12D);
    pub const OLYMPUS_LANGUAGE: Self = Self(0xD12E);
    pub const OLYMPUS_LANGUAGE_CODE: Self = Self(0xD12F);
    pub const CANON_EOS_COMPRESSION_S: Self = Self(0xD130);
    pub const NIKON_HDR_MODE: Self = Self(0xD130);
    pub const OLYMPUS_RECVIEW_MODE: Self = Self(0xD130);
    pub const CANON_EOS_COMPRESSION_M1: Self = Self(0xD131);
    pub const MTP_PLAYS_FOR_SURE_ID: Self = Self(0xD131);
    pub const NIKON_HDR_HIGH_DYNAMIC: Self = Self(0xD131);
    pub const OLYMPUS_SLEEP_TIME: Self = Self(0xD131);
    pub const CANON_EOS_COMPRESSION_M2: Self = Self(0xD132);
    pub const MTP_ZUNE_UNKNOWN2: Self = Self(0xD132);
    pub const NIKON_HDR_SMOOTHING: Self = Self(0xD132);
    pub const OLYMPUS_MANUAL_WBGM_BIAS: Self = Self(0xD132);
    pub const CANON_EOS_COMPRESSION_L: Self = Self(0xD133);
    pub const OLYMPUS_AELAFL_MODE: Self = Self(0xD135);
    pub const OLYMPUS_AEL_BUTTON_STATUS: Self = Self(0xD136);
    pub const OLYMPUS_COMPRESSION_SETTING_EX: Self = Self(0xD137);
    pub const OLYMPUS_TONE_MODE: Self = Self(0xD139);
    pub const OLYMPUS_GRADATION_MODE: Self = Self(0xD13A);
    pub const OLYMPUS_DEVELOP_MODE: Self = Self(0xD13B);
    pub const OLYMPUS_EXTEND_INNER_FLASH_MODE: Self = Self(0xD13C);
    pub const OLYMPUS_OUTPUT_DEVICE_MODE: Self = Self(0xD13D);
    pub const OLYMPUS_LIVE_VIEW_MODE: Self = Self(0xD13E);
    pub const CANON_EOS_PC_WHITE_BALANCE1: Self = Self(0xD140);
    pub const NIKON_OPTIMIZE_IMAGE: Self = Self(0xD140);
    pub const OLYMPUS_LCD_BACKLIGHT: Self = Self(0xD140);
    pub const CANON_EOS_PC_WHITE_BALANCE2: Self = Self(0xD141);
    pub const OLYMPUS_CUSTOM_DEVELOP: Self = Self(0xD141);
    pub const CANON_EOS_PC_WHITE_BALANCE3: Self = Self(0xD142);
    pub const NIKON_SATURATION: Self = Self(0xD142);
    pub const OLYMPUS_GRADATION_AUTO_BIAS: Self = Self(0xD142);
    pub const CANON_EOS_PC_WHITE_BALANCE4: Self = Self(0xD143);
    pub const NIKON_BW_FILLER_EFFECT: Self = Self(0xD143);
    pub const OLYMPUS_FLASH_RC_MODE: Self = Self(0xD143);
    pub const CANON_EOS_PC_WHITE_BALANCE5: Self = Self(0xD144);
    pub const NIKON_BW_SHARPNESS: Self = Self(0xD144);
    pub const OLYMPUS_FLASH_RC_GROUP_VALUE: Self = Self(0xD144);
    pub const CANON_EOS_M_WHITE_BALANCE: Self = Self(0xD145);
    pub const NIKON_BW_CONTRAST: Self = Self(0xD145);
    pub const OLYMPUS_FLASH_RC_CHANNEL_VALUE: Self = Self(0xD145);
    pub const NIKON_BW_SETTING_TYPE: Self = Self(0xD146);
    pub const OLYMPUS_FLASH_RCFP_MODE: Self = Self(0xD146);
    pub const OLYMPUS_FLASH_RC_PHOTO_CHROMIC_MODE: Self = Self(0xD147);
    pub const NIKON_SLOT2SAVE_MODE: Self = Self(0xD148);
    pub const OLYMPUS_FLASH_RC_PHOTO_CHROMIC_BIAS: Self = Self(0xD148);
    pub const NIKON_RAW_BIT_MODE: Self = Self(0xD149);
    pub const OLYMPUS_FLASH_RC_PHOTO_CHROMIC_MANUAL_BIAS: Self = Self(0xD149);
    pub const OLYMPUS_FLASH_RC_QUANTITY_LIGHT_LEVEL: Self = Self(0xD14A);
    pub const OLYMPUS_FOCUS_METERING_VALUE: Self = Self(0xD14B);
    pub const OLYMPUS_ISO_BRACKETING_FRAME: Self = Self(0xD14C);
    pub const OLYMPUS_ISO_BRACKETING_STEP: Self = Self(0xD14D);
    pub const NIKON_ISO_AUTO_TIME: Self = Self(0xD14E);
    pub const OLYMPUS_BULB_MF_MODE: Self = Self(0xD14E);
    pub const NIKON_FLOURESCENT_TYPE: Self = Self(0xD14F);
    pub const OLYMPUS_BURST_FPS_VALUE: Self = Self(0xD14F);
    pub const CANON_EOS_PICTURE_STYLE_STANDARD: Self = Self(0xD150);
    pub const NIKON_TUNE_COLOUR_TEMPERATURE: Self = Self(0xD150);
    pub const OLYMPUS_ISO_AUTO_BASE_VALUE: Self = Self(0xD150);
    pub const CANON_EOS_PICTURE_STYLE_PORTRAIT: Self = Self(0xD151);
    pub const NIKON_TUNE_PRESET0: Self = Self(0xD151);
    pub const OLYMPUS_ISO_AUTO_MAX_VALUE: Self = Self(0xD151);
    pub const CANON_EOS_PICTURE_STYLE_LANDSCAPE: Self = Self(0xD152);
    pub const NIKON_TUNE_PRESET1: Self = Self(0xD152);
    pub const OLYMPUS_BULB_LIMITER_VALUE: Self = Self(0xD152);
    pub const CANON_EOS_PICTURE_STYLE_NEUTRAL: Self = Self(0xD153);
    pub const NIKON_TUNE_PRESET2: Self = Self(0xD153);
    pub const OLYMPUS_DPI_MODE: Self = Self(0xD153);
    pub const CANON_EOS_PICTURE_STYLE_FAITHFUL: Self = Self(0xD154);
    pub const NIKON_TUNE_PRESET3: Self = Self(0xD154);
    pub const OLYMPUS_DPI_CUSTOM_VALUE: Self = Self(0xD154);
    pub const CANON_EOS_PICTURE_STYLE_BLACK_WHITE: Self = Self(0xD155);
    pub const NIKON_TUNE_PRESET4: Self = Self(0xD155);
    pub const OLYMPUS_RESOLUTION_VALUE_SETTING: Self = Self(0xD155);
    pub const OLYMPUS_AF_TARGET_SIZE: Self = Self(0xD157);
    pub const OLYMPUS_LIGHT_SENSOR_MODE: Self = Self(0xD158);
    pub const OLYMPUS_AE_BRACKET: Self = Self(0xD159);
    pub const OLYMPUS_WBRB_BRACKET: Self = Self(0xD15A);
    pub const OLYMPUS_WBGM_BRACKET: Self = Self(0xD15B);
    pub const OLYMPUS_FLASH_BRACKET: Self = Self(0xD15C);
    pub const OLYMPUS_ISO_BRACKET: Self = Self(0xD15D);
    pub const OLYMPUS_MY_MODE_STATUS: Self = Self(0xD15E);
    pub const CANON_EOS_PICTURE_STYLE_USER_SET1: Self = Self(0xD160);
    pub const NIKON_BEEP_OFF: Self = Self(0xD160);
    pub const CANON_EOS_PICTURE_STYLE_USER_SET2: Self = Self(0xD161);
    pub const NIKON_AUTOFOCUS_MODE: Self = Self(0xD161);
    pub const CANON_EOS_PICTURE_STYLE_USER_SET3: Self = Self(0xD162);
    pub const NIKON_AF_ASSIST: Self = Self(0xD163);
    pub const NIKON_IMAGE_REVIEW: Self = Self(0xD165);
    pub const NIKON_AF_AREA_ILLUMINATION: Self = Self(0xD166);
    pub const NIKON_FLASH_MODE: Self = Self(0xD167);
    pub const NIKON_FLASH_COMMANDER_MODE: Self = Self(0xD168);
    pub const NIKON_FLASH_SIGN: Self = Self(0xD169);
    pub const NIKON_ISO_AUTO_ALT: Self = Self(0xD16A);
    pub const NIKON_REMOTE_TIMEOUT: Self = Self(0xD16B);
    pub const NIKON_GRID_DISPLAY: Self = Self(0xD16C);
    pub const NIKON_FLASH_MODE_MANUAL_POWER: Self = Self(0xD16D);
    pub const NIKON_FLASH_MODE_COMMANDER_POWER: Self = Self(0xD16E);
    pub const NIKON_AUTO_FP: Self = Self(0xD16F);
    pub const CANON_EOS_PICTURE_STYLE_PARAM1: Self = Self(0xD170);
    pub const CANON_EOS_PICTURE_STYLE_PARAM2: Self = Self(0xD171);
    pub const CANON_EOS_PICTURE_STYLE_PARAM3: Self = Self(0xD172);
    pub const CANON_EOS_FLAVOR_LUT_PARAMS: Self = Self(0xD17F);
    pub const CANON_EOS_CUSTOM_FUNC1: Self = Self(0xD180);
    pub const NIKON_CSM_MENU: Self = Self(0xD180);
    pub const CANON_EOS_CUSTOM_FUNC2: Self = Self(0xD181);
    pub const MTP_ZUNE_UNKNOWN1: Self = Self(0xD181);
    pub const MTP_ZUNE_UNKNOWN_VERSION: Self = Self(0xD181);
    pub const NIKON_WARNING_DISPLAY: Self = Self(0xD181);
    pub const CANON_EOS_CUSTOM_FUNC3: Self = Self(0xD182);
    pub const NIKON_BATTERY_CELL_KIND: Self = Self(0xD182);
    pub const CANON_EOS_CUSTOM_FUNC4: Self = Self(0xD183);
    pub const NIKON_ISO_AUTO_HI_LIMIT: Self = Self(0xD183);
    pub const CANON_EOS_CUSTOM_FUNC5: Self = Self(0xD184);
    pub const NIKON_DYNAMIC_AF_AREA: Self = Self(0xD184);
    pub const CANON_EOS_CUSTOM_FUNC6: Self = Self(0xD185);
    pub const CANON_EOS_CUSTOM_FUNC7: Self = Self(0xD186);
    pub const NIKON_CONTINUOUS_SPEED_HIGH: Self = Self(0xD186);
    pub const CANON_EOS_CUSTOM_FUNC8: Self = Self(0xD187);
    pub const NIKON_INFO_DISP_SETTING: Self = Self(0xD187);
    pub const CANON_EOS_CUSTOM_FUNC9: Self = Self(0xD188);
    pub const CANON_EOS_CUSTOM_FUNC10: Self = Self(0xD189);
    pub const NIKON_PREVIEW_BUTTON: Self = Self(0xD189);
    pub const NIKON_PREVIEW_BUTTON2: Self = Self(0xD18A);
    pub const NIKON_AEAF_LOCK_BUTTON2: Self = Self(0xD18B);
    pub const NIKON_INDICATOR_DISP: Self = Self(0xD18D);
    pub const NIKON_CELL_KIND_PRIORITY: Self = Self(0xD18E);
    pub const CANON_EOS_CUSTOM_FUNC11: Self = Self(0xD18A);
    pub const CANON_EOS_CUSTOM_FUNC12: Self = Self(0xD18B);
    pub const CANON_EOS_CUSTOM_FUNC13: Self = Self(0xD18C);
    pub const CANON_EOS_CUSTOM_FUNC14: Self = Self(0xD18D);
    pub const CANON_EOS_CUSTOM_FUNC15: Self = Self(0xD18E);
    pub const CANON_EOS_CUSTOM_FUNC16: Self = Self(0xD18F);
    pub const CANON_EOS_CUSTOM_FUNC17: Self = Self(0xD190);
    pub const NIKON_BRACKETING_FRAMES_AND_STEPS: Self = Self(0xD190);
    pub const CANON_EOS_CUSTOM_FUNC18: Self = Self(0xD191);
    pub const CANON_EOS_CUSTOM_FUNC19: Self = Self(0xD192);
    pub const NIKON_LIVE_VIEW_MODE: Self = Self(0xD1A0);
    pub const NIKON_LIVE_VIEW_DRIVE_MODE: Self = Self(0xD1A1);
    pub const NIKON_LIVE_VIEW_STATUS: Self = Self(0xD1A2);
    pub const NIKON_LIVE_VIEW_IMAGE_ZOOM_RATIO: Self = Self(0xD1A3);
    pub const NIKON_LIVE_VIEW_PROHIBIT_CONDITION: Self = Self(0xD1A4);
    pub const NIKON_EXPOSURE_DISPLAY_STATUS: Self = Self(0xD1B0);
    pub const NIKON_EXPOSURE_INDICATE_STATUS: Self = Self(0xD1B1);
    pub const NIKON_INFO_DISP_ERR_STATUS: Self = Self(0xD1B2);
    pub const NIKON_EXPOSURE_INDICATE_LIGHTUP: Self = Self(0xD1B3);
    pub const NIKON_FLASH_OPEN: Self = Self(0xD1C0);
    pub const NIKON_FLASH_CHARGED: Self = Self(0xD1C1);
    pub const NIKON_FLASH_M_REPEAT_VALUE: Self = Self(0xD1D0);
    pub const NIKON_FLASH_M_REPEAT_COUNT: Self = Self(0xD1D1);
    pub const NIKON_FLASH_M_REPEAT_INTERVAL: Self = Self(0xD1D2);
    pub const NIKON_FLASH_COMMAND_CHANNEL: Self = Self(0xD1D3);
    pub const NIKON_FLASH_COMMAND_SELF_MODE: Self = Self(0xD1D4);
    pub const NIKON_FLASH_COMMAND_SELF_COMPENSATION: Self = Self(0xD1D5);
    pub const NIKON_FLASH_COMMAND_SELF_VALUE: Self = Self(0xD1D6);
    pub const NIKON_FLASH_COMMAND_A_MODE: Self = Self(0xD1D7);
    pub const NIKON_FLASH_COMMAND_A_COMPENSATION: Self = Self(0xD1D8);
    pub const NIKON_FLASH_COMMAND_A_VALUE: Self = Self(0xD1D9);
    pub const NIKON_FLASH_COMMAND_B_MODE: Self = Self(0xD1DA);
    pub const NIKON_FLASH_COMMAND_B_COMPENSATION: Self = Self(0xD1DB);
    pub const NIKON_FLASH_COMMAND_B_VALUE: Self = Self(0xD1DC);
    pub const CANON_EOS_CUSTOM_FUNC_EX: Self = Self(0xD1A0);
    pub const CANON_EOS_MY_MENU: Self = Self(0xD1A1);
    pub const CANON_EOS_MY_MENU_LIST: Self = Self(0xD1A2);
    pub const CANON_EOS_WFT_STATUS: Self = Self(0xD1A3);
    pub const CANON_EOS_WFT_INPUT_TRANSMISSION: Self = Self(0xD1A4);
    pub const CANON_EOS_HD_DIRECTORY_STRUCTURE: Self = Self(0xD1A5);
    pub const CANON_EOS_BATTERY_INFO: Self = Self(0xD1A6);
    pub const CANON_EOS_ADAPTER_INFO: Self = Self(0xD1A7);
    pub const CANON_EOS_LENS_STATUS: Self = Self(0xD1A8);
    pub const CANON_EOS_QUICK_REVIEW_TIME: Self = Self(0xD1A9);
    pub const CANON_EOS_CARD_EXTENSION: Self = Self(0xD1AA);
    pub const CANON_EOS_TEMP_STATUS: Self = Self(0xD1AB);
    pub const CANON_EOS_SHUTTER_COUNTER: Self = Self(0xD1AC);
    pub const CANON_EOS_SPECIAL_OPTION: Self = Self(0xD1AD);
    pub const CANON_EOS_PHOTO_STUDIO_MODE: Self = Self(0xD1AE);
    pub const CANON_EOS_SERIAL_NUMBER: Self = Self(0xD1AF);
    pub const CANON_EOS_EVF_OUTPUT_DEVICE: Self = Self(0xD1B0);
    pub const CANON_EOS_EVF_MODE: Self = Self(0xD1B1);
    pub const CANON_EOS_DEPTH_OF_FIELD_PREVIEW: Self = Self(0xD1B2);
    pub const CANON_EOS_EVF_SHARPNESS: Self = Self(0xD1B3);
    pub const CANON_EOS_EVFWB_MODE: Self = Self(0xD1B4);
    pub const CANON_EOS_EVF_CLICK_WB_COEFFS: Self = Self(0xD1B5);
    pub const CANON_EOS_EVF_COLOR_TEMP: Self = Self(0xD1B6);
    pub const CANON_EOS_EXPOSURE_SIM_MODE: Self = Self(0xD1B7);
    pub const CANON_EOS_EVF_RECORD_STATUS: Self = Self(0xD1B8);
    pub const CANON_EOS_LV_AF_SYSTEM: Self = Self(0xD1BA);
    pub const CANON_EOS_MOV_SIZE: Self = Self(0xD1BB);
    pub const CANON_EOS_LV_VIEW_TYPE_SELECT: Self = Self(0xD1BC);
    pub const CANON_EOS_ARTIST: Self = Self(0xD1D0);
    pub const CANON_EOS_COPYRIGHT: Self = Self(0xD1D1);
    pub const CANON_EOS_BRACKET_VALUE: Self = Self(0xD1D2);
    pub const CANON_EOS_FOCUS_INFO_EX: Self = Self(0xD1D3);
    pub const CANON_EOS_DEPTH_OF_FIELD: Self = Self(0xD1D4);
    pub const CANON_EOS_BRIGHTNESS: Self = Self(0xD1D5);
    pub const CANON_EOS_LENS_ADJUST_PARAMS: Self = Self(0xD1D6);
    pub const CANON_EOS_EF_COMP: Self = Self(0xD1D7);
    pub const CANON_EOS_LENS_NAME: Self = Self(0xD1D8);
    pub const CANON_EOS_AEB: Self = Self(0xD1D9);
    pub const CANON_EOS_STROBO_SETTING: Self = Self(0xD1DA);
    pub const CANON_EOS_STROBO_WIRELESS_SETTING: Self = Self(0xD1DB);
    pub const CANON_EOS_STROBO_FIRING: Self = Self(0xD1DC);
    pub const CANON_EOS_LENS_ID: Self = Self(0xD1DD);
    pub const NIKON_ACTIVE_PIC_CTRL_ITEM: Self = Self(0xD200);
    pub const FUJI_RELEASE_MODE: Self = Self(0xD201);
    pub const NIKON_CHANGE_PIC_CTRL_ITEM: Self = Self(0xD201);
    pub const FUJI_FOCUS_AREAS: Self = Self(0xD206);
    pub const FUJI_AE_LOCK: Self = Self(0xD213);
    pub const MTP_ZUNE_UNKNOWN3: Self = Self(0xD215);
    pub const MTP_ZUNE_UNKNOWN4: Self = Self(0xD216);
    pub const FUJI_APERTURE: Self = Self(0xD218);
    pub const FUJI_SHUTTER_SPEED: Self = Self(0xD219);
    pub const MTP_SYNCHRONIZATION_PARTNER: Self = Self(0xD401);
    pub const MTP_DEVICE_FRIENDLY_NAME: Self = Self(0xD402);
    pub const MTP_VOLUME_LEVEL: Self = Self(0xD403);
    pub const MTP_DEVICE_ICON: Self = Self(0xD405);
    pub const MTP_SESSION_INITIATOR_INFO: Self = Self(0xD406);
    pub const MTP_PERCEIVED_DEVICE_TYPE: Self = Self(0xD407);
    pub const MTP_PLAYBACK_RATE: Self = Self(0xD410);
    pub const MTP_PLAYBACK_OBJECT: Self = Self(0xD411);
    pub const MTP_PLAYBACK_CONTAINER_INDEX: Self = Self(0xD412);
    pub const MTP_PLAYBACK_POSITION: Self = Self(0xD413);
    pub const EXTENSION_MASK: Self = Self(0xF000);

    /// Debug name from go-mtpfs `DPC_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x5000 => "Undefined",
            0x5001 => "BatteryLevel",
            0x5002 => "FunctionalMode",
            0x5003 => "ImageSize",
            0x5004 => "CompressionSetting",
            0x5005 => "WhiteBalance",
            0x5006 => "RGBGain",
            0x5007 => "FNumber",
            0x5008 => "FocalLength",
            0x5009 => "FocusDistance",
            0x500A => "FocusMode",
            0x500B => "ExposureMeteringMode",
            0x500C => "FlashMode",
            0x500D => "ExposureTime",
            0x500E => "ExposureProgramMode",
            0x500F => "ExposureIndex",
            0x5010 => "ExposureBiasCompensation",
            0x5011 => "DateTime",
            0x5012 => "CaptureDelay",
            0x5013 => "StillCaptureMode",
            0x5014 => "Contrast",
            0x5015 => "Sharpness",
            0x5016 => "DigitalZoom",
            0x5017 => "EffectMode",
            0x5018 => "BurstNumber",
            0x5019 => "BurstInterval",
            0x501A => "TimelapseNumber",
            0x501B => "TimelapseInterval",
            0x501C => "FocusMeteringMode",
            0x501D => "UploadURL",
            0x501E => "Artist",
            0x501F => "CopyrightInfo",
            0x5020 => "SupportedStreams",
            0x5021 => "EnabledStreams",
            0x5022 => "VideoFormat",
            0x5023 => "VideoResolution",
            0x5024 => "VideoQuality",
            0x5025 => "VideoFrameRate",
            0x5026 => "VideoContrast",
            0x5027 => "VideoBrightness",
            0x5028 => "AudioFormat",
            0x5029 => "AudioBitrate",
            0x502A => "AudioSamplingRate",
            0x502B => "AudioBitPerSample",
            0x502C => "AudioVolume",
            0xD000 => "EXTENSION",
            0xD080 => "CASIO_UNKNOWN_18",
            0xD118 => "OLYMPUS_FLBracketingFrame",
            0xD127 => "OLYMPUS_NoiseReductionRandom",
            0xD129 => "OLYMPUS_ShadingMode",
            0xD12A => "OLYMPUS_ISOBoostMode",
            0xD12B => "OLYMPUS_ExposureIndexBiasStep",
            0xD12C => "OLYMPUS_FilterEffect",
            0xD12D => "OLYMPUS_ColorTune",
            0xD12E => "OLYMPUS_Language",
            0xD12F => "OLYMPUS_LanguageCode",
            0xD135 => "OLYMPUS_AELAFLMode",
            0xD136 => "OLYMPUS_AELButtonStatus",
            0xD137 => "OLYMPUS_CompressionSettingEx",
            0xD139 => "OLYMPUS_ToneMode",
            0xD13A => "OLYMPUS_GradationMode",
            0xD13B => "OLYMPUS_DevelopMode",
            0xD13C => "OLYMPUS_ExtendInnerFlashMode",
            0xD13D => "OLYMPUS_OutputDeviceMode",
            0xD13E => "OLYMPUS_LiveViewMode",
            0xD147 => "OLYMPUS_FlashRCPhotoChromicMode",
            0xD14A => "OLYMPUS_FlashRCQuantityLightLevel",
            0xD14B => "OLYMPUS_FocusMeteringValue",
            0xD14C => "OLYMPUS_ISOBracketingFrame",
            0xD14D => "OLYMPUS_ISOBracketingStep",
            0xD157 => "OLYMPUS_AFTargetSize",
            0xD158 => "OLYMPUS_LightSensorMode",
            0xD159 => "OLYMPUS_AEBracket",
            0xD15A => "OLYMPUS_WBRBBracket",
            0xD15B => "OLYMPUS_WBGMBracket",
            0xD15C => "OLYMPUS_FlashBracket",
            0xD15D => "OLYMPUS_ISOBracket",
            0xD15E => "OLYMPUS_MyModeStatus",
            0xD215 => "MTP_ZUNE_UNKNOWN3",
            0xD216 => "MTP_ZUNE_UNKNOWN4",
            0xD401 => "MTP_SynchronizationPartner",
            0xD402 => "MTP_DeviceFriendlyName",
            0xD403 => "MTP_VolumeLevel",
            0xD405 => "MTP_DeviceIcon",
            0xD406 => "MTP_SessionInitiatorInfo",
            0xD407 => "MTP_PerceivedDeviceType",
            0xD410 => "MTP_PlaybackRate",
            0xD411 => "MTP_PlaybackObject",
            0xD412 => "MTP_PlaybackContainerIndex",
            0xD413 => "MTP_PlaybackPosition",
            0xF000 => "EXTENSION_MASK",
            _ => return None,
        })
    }
}

impl fmt::Display for DevicePropCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// object property code (`OPC_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ObjectPropCode(pub u16);

impl ObjectPropCode {
    pub const WIRELESS_CONFIGURATION_FILE: Self = Self(0xB104);
    pub const BUY_FLAG: Self = Self(0xD901);
    pub const STORAGE_ID: Self = Self(0xDC01);
    pub const OBJECT_FORMAT: Self = Self(0xDC02);
    pub const PROTECTION_STATUS: Self = Self(0xDC03);
    pub const OBJECT_SIZE: Self = Self(0xDC04);
    pub const ASSOCIATION_TYPE: Self = Self(0xDC05);
    pub const ASSOCIATION_DESC: Self = Self(0xDC06);
    pub const OBJECT_FILE_NAME: Self = Self(0xDC07);
    pub const DATE_CREATED: Self = Self(0xDC08);
    pub const DATE_MODIFIED: Self = Self(0xDC09);
    pub const KEYWORDS: Self = Self(0xDC0A);
    pub const PARENT_OBJECT: Self = Self(0xDC0B);
    pub const ALLOWED_FOLDER_CONTENTS: Self = Self(0xDC0C);
    pub const HIDDEN: Self = Self(0xDC0D);
    pub const SYSTEM_OBJECT: Self = Self(0xDC0E);
    pub const PERSISTANT_UNIQUE_OBJECT_IDENTIFIER: Self = Self(0xDC41);
    pub const SYNC_ID: Self = Self(0xDC42);
    pub const PROPERTY_BAG: Self = Self(0xDC43);
    pub const NAME: Self = Self(0xDC44);
    pub const CREATED_BY: Self = Self(0xDC45);
    pub const ARTIST: Self = Self(0xDC46);
    pub const DATE_AUTHORED: Self = Self(0xDC47);
    pub const DESCRIPTION: Self = Self(0xDC48);
    pub const URL_REFERENCE: Self = Self(0xDC49);
    pub const LANGUAGE_LOCALE: Self = Self(0xDC4A);
    pub const COPYRIGHT_INFORMATION: Self = Self(0xDC4B);
    pub const SOURCE: Self = Self(0xDC4C);
    pub const ORIGIN_LOCATION: Self = Self(0xDC4D);
    pub const DATE_ADDED: Self = Self(0xDC4E);
    pub const NON_CONSUMABLE: Self = Self(0xDC4F);
    pub const CORRUPT_OR_UNPLAYABLE: Self = Self(0xDC50);
    pub const PRODUCER_SERIAL_NUMBER: Self = Self(0xDC51);
    pub const REPRESENTATIVE_SAMPLE_FORMAT: Self = Self(0xDC81);
    pub const REPRESENTATIVE_SAMPLE_SIZE: Self = Self(0xDC82);
    pub const REPRESENTATIVE_SAMPLE_HEIGHT: Self = Self(0xDC83);
    pub const REPRESENTATIVE_SAMPLE_WIDTH: Self = Self(0xDC84);
    pub const REPRESENTATIVE_SAMPLE_DURATION: Self = Self(0xDC85);
    pub const REPRESENTATIVE_SAMPLE_DATA: Self = Self(0xDC86);
    pub const WIDTH: Self = Self(0xDC87);
    pub const HEIGHT: Self = Self(0xDC88);
    pub const DURATION: Self = Self(0xDC89);
    pub const RATING: Self = Self(0xDC8A);
    pub const TRACK: Self = Self(0xDC8B);
    pub const GENRE: Self = Self(0xDC8C);
    pub const CREDITS: Self = Self(0xDC8D);
    pub const LYRICS: Self = Self(0xDC8E);
    pub const SUBSCRIPTION_CONTENT_ID: Self = Self(0xDC8F);
    pub const PRODUCED_BY: Self = Self(0xDC90);
    pub const USE_COUNT: Self = Self(0xDC91);
    pub const SKIP_COUNT: Self = Self(0xDC92);
    pub const LAST_ACCESSED: Self = Self(0xDC93);
    pub const PARENTAL_RATING: Self = Self(0xDC94);
    pub const META_GENRE: Self = Self(0xDC95);
    pub const COMPOSER: Self = Self(0xDC96);
    pub const EFFECTIVE_RATING: Self = Self(0xDC97);
    pub const SUBTITLE: Self = Self(0xDC98);
    pub const ORIGINAL_RELEASE_DATE: Self = Self(0xDC99);
    pub const ALBUM_NAME: Self = Self(0xDC9A);
    pub const ALBUM_ARTIST: Self = Self(0xDC9B);
    pub const MOOD: Self = Self(0xDC9C);
    pub const DRM_STATUS: Self = Self(0xDC9D);
    pub const SUB_DESCRIPTION: Self = Self(0xDC9E);
    pub const IS_CROPPED: Self = Self(0xDCD1);
    pub const IS_COLOR_CORRECTED: Self = Self(0xDCD2);
    pub const IMAGE_BIT_DEPTH: Self = Self(0xDCD3);
    pub const FNUMBER: Self = Self(0xDCD4);
    pub const EXPOSURE_TIME: Self = Self(0xDCD5);
    pub const EXPOSURE_INDEX: Self = Self(0xDCD6);
    pub const DISPLAY_NAME: Self = Self(0xDCE0);
    pub const BODY_TEXT: Self = Self(0xDCE1);
    pub const SUBJECT: Self = Self(0xDCE2);
    pub const PRIORITY: Self = Self(0xDCE3);
    pub const GIVEN_NAME: Self = Self(0xDD00);
    pub const MIDDLE_NAMES: Self = Self(0xDD01);
    pub const FAMILY_NAME: Self = Self(0xDD02);
    pub const PREFIX: Self = Self(0xDD03);
    pub const SUFFIX: Self = Self(0xDD04);
    pub const PHONETIC_GIVEN_NAME: Self = Self(0xDD05);
    pub const PHONETIC_FAMILY_NAME: Self = Self(0xDD06);
    pub const EMAIL_PRIMARY: Self = Self(0xDD07);
    pub const EMAIL_PERSONAL1: Self = Self(0xDD08);
    pub const EMAIL_PERSONAL2: Self = Self(0xDD09);
    pub const EMAIL_BUSINESS1: Self = Self(0xDD0A);
    pub const EMAIL_BUSINESS2: Self = Self(0xDD0B);
    pub const EMAIL_OTHERS: Self = Self(0xDD0C);
    pub const PHONE_NUMBER_PRIMARY: Self = Self(0xDD0D);
    pub const PHONE_NUMBER_PERSONAL: Self = Self(0xDD0E);
    pub const PHONE_NUMBER_PERSONAL2: Self = Self(0xDD0F);
    pub const PHONE_NUMBER_BUSINESS: Self = Self(0xDD10);
    pub const PHONE_NUMBER_BUSINESS2: Self = Self(0xDD11);
    pub const PHONE_NUMBER_MOBILE: Self = Self(0xDD12);
    pub const PHONE_NUMBER_MOBILE2: Self = Self(0xDD13);
    pub const FAX_NUMBER_PRIMARY: Self = Self(0xDD14);
    pub const FAX_NUMBER_PERSONAL: Self = Self(0xDD15);
    pub const FAX_NUMBER_BUSINESS: Self = Self(0xDD16);
    pub const PAGER_NUMBER: Self = Self(0xDD17);
    pub const PHONE_NUMBER_OTHERS: Self = Self(0xDD18);
    pub const PRIMARY_WEB_ADDRESS: Self = Self(0xDD19);
    pub const PERSONAL_WEB_ADDRESS: Self = Self(0xDD1A);
    pub const BUSINESS_WEB_ADDRESS: Self = Self(0xDD1B);
    pub const INSTANT_MESSENGER_ADDRESS: Self = Self(0xDD1C);
    pub const INSTANT_MESSENGER_ADDRESS2: Self = Self(0xDD1D);
    pub const INSTANT_MESSENGER_ADDRESS3: Self = Self(0xDD1E);
    pub const POSTAL_ADDRESS_PERSONAL_FULL: Self = Self(0xDD1F);
    pub const POSTAL_ADDRESS_PERSONAL_FULL_LINE1: Self = Self(0xDD20);
    pub const POSTAL_ADDRESS_PERSONAL_FULL_LINE2: Self = Self(0xDD21);
    pub const POSTAL_ADDRESS_PERSONAL_FULL_CITY: Self = Self(0xDD22);
    pub const POSTAL_ADDRESS_PERSONAL_FULL_REGION: Self = Self(0xDD23);
    pub const POSTAL_ADDRESS_PERSONAL_FULL_POSTAL_CODE: Self = Self(0xDD24);
    pub const POSTAL_ADDRESS_PERSONAL_FULL_COUNTRY: Self = Self(0xDD25);
    pub const POSTAL_ADDRESS_BUSINESS_FULL: Self = Self(0xDD26);
    pub const POSTAL_ADDRESS_BUSINESS_LINE1: Self = Self(0xDD27);
    pub const POSTAL_ADDRESS_BUSINESS_LINE2: Self = Self(0xDD28);
    pub const POSTAL_ADDRESS_BUSINESS_CITY: Self = Self(0xDD29);
    pub const POSTAL_ADDRESS_BUSINESS_REGION: Self = Self(0xDD2A);
    pub const POSTAL_ADDRESS_BUSINESS_POSTAL_CODE: Self = Self(0xDD2B);
    pub const POSTAL_ADDRESS_BUSINESS_COUNTRY: Self = Self(0xDD2C);
    pub const POSTAL_ADDRESS_OTHER_FULL: Self = Self(0xDD2D);
    pub const POSTAL_ADDRESS_OTHER_LINE1: Self = Self(0xDD2E);
    pub const POSTAL_ADDRESS_OTHER_LINE2: Self = Self(0xDD2F);
    pub const POSTAL_ADDRESS_OTHER_CITY: Self = Self(0xDD30);
    pub const POSTAL_ADDRESS_OTHER_REGION: Self = Self(0xDD31);
    pub const POSTAL_ADDRESS_OTHER_POSTAL_CODE: Self = Self(0xDD32);
    pub const POSTAL_ADDRESS_OTHER_COUNTRY: Self = Self(0xDD33);
    pub const ORGANIZATION_NAME: Self = Self(0xDD34);
    pub const PHONETIC_ORGANIZATION_NAME: Self = Self(0xDD35);
    pub const ROLE: Self = Self(0xDD36);
    pub const BIRTHDATE: Self = Self(0xDD37);
    pub const MESSAGE_TO: Self = Self(0xDD40);
    pub const MESSAGE_CC: Self = Self(0xDD41);
    pub const MESSAGE_BCC: Self = Self(0xDD42);
    pub const MESSAGE_READ: Self = Self(0xDD43);
    pub const MESSAGE_RECEIVED_TIME: Self = Self(0xDD44);
    pub const MESSAGE_SENDER: Self = Self(0xDD45);
    pub const ACTIVITY_BEGIN_TIME: Self = Self(0xDD50);
    pub const ACTIVITY_END_TIME: Self = Self(0xDD51);
    pub const ACTIVITY_LOCATION: Self = Self(0xDD52);
    pub const ACTIVITY_REQUIRED_ATTENDEES: Self = Self(0xDD54);
    pub const ACTIVITY_OPTIONAL_ATTENDEES: Self = Self(0xDD55);
    pub const ACTIVITY_RESOURCES: Self = Self(0xDD56);
    pub const ACTIVITY_ACCEPTED: Self = Self(0xDD57);
    pub const OWNER: Self = Self(0xDD5D);
    pub const EDITOR: Self = Self(0xDD5E);
    pub const WEBMASTER: Self = Self(0xDD5F);
    pub const URL_SOURCE: Self = Self(0xDD60);
    pub const URL_DESTINATION: Self = Self(0xDD61);
    pub const TIME_BOOKMARK: Self = Self(0xDD62);
    pub const OBJECT_BOOKMARK: Self = Self(0xDD63);
    pub const BYTE_BOOKMARK: Self = Self(0xDD64);
    pub const LAST_BUILD_DATE: Self = Self(0xDD70);
    pub const TIMETO_LIVE: Self = Self(0xDD71);
    pub const MEDIA_GUID: Self = Self(0xDD72);
    pub const TOTAL_BIT_RATE: Self = Self(0xDE91);
    pub const BIT_RATE_TYPE: Self = Self(0xDE92);
    pub const SAMPLE_RATE: Self = Self(0xDE93);
    pub const NUMBER_OF_CHANNELS: Self = Self(0xDE94);
    pub const AUDIO_BIT_DEPTH: Self = Self(0xDE95);
    pub const SCAN_DEPTH: Self = Self(0xDE97);
    pub const AUDIO_WAVE_CODEC: Self = Self(0xDE99);
    pub const AUDIO_BIT_RATE: Self = Self(0xDE9A);
    pub const VIDEO_FOUR_CC_CODEC: Self = Self(0xDE9B);
    pub const VIDEO_BIT_RATE: Self = Self(0xDE9C);
    pub const FRAMES_PER_THOUSAND_SECONDS: Self = Self(0xDE9D);
    pub const KEY_FRAME_DISTANCE: Self = Self(0xDE9E);
    pub const BUFFER_SIZE: Self = Self(0xDE9F);
    pub const ENCODING_QUALITY: Self = Self(0xDEA0);
    pub const ENCODING_PROFILE: Self = Self(0xDEA1);

    /// Debug name from go-mtpfs `OPC_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0xB104 => "WirelessConfigurationFile",
            0xD901 => "BuyFlag",
            0xDC01 => "StorageID",
            0xDC02 => "ObjectFormat",
            0xDC03 => "ProtectionStatus",
            0xDC04 => "ObjectSize",
            0xDC05 => "AssociationType",
            0xDC06 => "AssociationDesc",
            0xDC07 => "ObjectFileName",
            0xDC08 => "DateCreated",
            0xDC09 => "DateModified",
            0xDC0A => "Keywords",
            0xDC0B => "ParentObject",
            0xDC0C => "AllowedFolderContents",
            0xDC0D => "Hidden",
            0xDC0E => "SystemObject",
            0xDC41 => "PersistantUniqueObjectIdentifier",
            0xDC42 => "SyncID",
            0xDC43 => "PropertyBag",
            0xDC44 => "Name",
            0xDC45 => "CreatedBy",
            0xDC46 => "Artist",
            0xDC47 => "DateAuthored",
            0xDC48 => "Description",
            0xDC49 => "URLReference",
            0xDC4A => "LanguageLocale",
            0xDC4B => "CopyrightInformation",
            0xDC4C => "Source",
            0xDC4D => "OriginLocation",
            0xDC4E => "DateAdded",
            0xDC4F => "NonConsumable",
            0xDC50 => "CorruptOrUnplayable",
            0xDC51 => "ProducerSerialNumber",
            0xDC81 => "RepresentativeSampleFormat",
            0xDC82 => "RepresentativeSampleSize",
            0xDC83 => "RepresentativeSampleHeight",
            0xDC84 => "RepresentativeSampleWidth",
            0xDC85 => "RepresentativeSampleDuration",
            0xDC86 => "RepresentativeSampleData",
            0xDC87 => "Width",
            0xDC88 => "Height",
            0xDC89 => "Duration",
            0xDC8A => "Rating",
            0xDC8B => "Track",
            0xDC8C => "Genre",
            0xDC8D => "Credits",
            0xDC8E => "Lyrics",
            0xDC8F => "SubscriptionContentID",
            0xDC90 => "ProducedBy",
            0xDC91 => "UseCount",
            0xDC92 => "SkipCount",
            0xDC93 => "LastAccessed",
            0xDC94 => "ParentalRating",
            0xDC95 => "MetaGenre",
            0xDC96 => "Composer",
            0xDC97 => "EffectiveRating",
            0xDC98 => "Subtitle",
            0xDC99 => "OriginalReleaseDate",
            0xDC9A => "AlbumName",
            0xDC9B => "AlbumArtist",
            0xDC9C => "Mood",
            0xDC9D => "DRMStatus",
            0xDC9E => "SubDescription",
            0xDCD1 => "IsCropped",
            0xDCD2 => "IsColorCorrected",
            0xDCD3 => "ImageBitDepth",
            0xDCD4 => "Fnumber",
            0xDCD5 => "ExposureTime",
            0xDCD6 => "ExposureIndex",
            0xDCE0 => "DisplayName",
            0xDCE1 => "BodyText",
            0xDCE2 => "Subject",
            0xDCE3 => "Priority",
            0xDD00 => "GivenName",
            0xDD01 => "MiddleNames",
            0xDD02 => "FamilyName",
            0xDD03 => "Prefix",
            0xDD04 => "Suffix",
            0xDD05 => "PhoneticGivenName",
            0xDD06 => "PhoneticFamilyName",
            0xDD07 => "EmailPrimary",
            0xDD08 => "EmailPersonal1",
            0xDD09 => "EmailPersonal2",
            0xDD0A => "EmailBusiness1",
            0xDD0B => "EmailBusiness2",
            0xDD0C => "EmailOthers",
            0xDD0D => "PhoneNumberPrimary",
            0xDD0E => "PhoneNumberPersonal",
            0xDD0F => "PhoneNumberPersonal2",
            0xDD10 => "PhoneNumberBusiness",
            0xDD11 => "PhoneNumberBusiness2",
            0xDD12 => "PhoneNumberMobile",
            0xDD13 => "PhoneNumberMobile2",
            0xDD14 => "FaxNumberPrimary",
            0xDD15 => "FaxNumberPersonal",
            0xDD16 => "FaxNumberBusiness",
            0xDD17 => "PagerNumber",
            0xDD18 => "PhoneNumberOthers",
            0xDD19 => "PrimaryWebAddress",
            0xDD1A => "PersonalWebAddress",
            0xDD1B => "BusinessWebAddress",
            0xDD1C => "InstantMessengerAddress",
            0xDD1D => "InstantMessengerAddress2",
            0xDD1E => "InstantMessengerAddress3",
            0xDD1F => "PostalAddressPersonalFull",
            0xDD20 => "PostalAddressPersonalFullLine1",
            0xDD21 => "PostalAddressPersonalFullLine2",
            0xDD22 => "PostalAddressPersonalFullCity",
            0xDD23 => "PostalAddressPersonalFullRegion",
            0xDD24 => "PostalAddressPersonalFullPostalCode",
            0xDD25 => "PostalAddressPersonalFullCountry",
            0xDD26 => "PostalAddressBusinessFull",
            0xDD27 => "PostalAddressBusinessLine1",
            0xDD28 => "PostalAddressBusinessLine2",
            0xDD29 => "PostalAddressBusinessCity",
            0xDD2A => "PostalAddressBusinessRegion",
            0xDD2B => "PostalAddressBusinessPostalCode",
            0xDD2C => "PostalAddressBusinessCountry",
            0xDD2D => "PostalAddressOtherFull",
            0xDD2E => "PostalAddressOtherLine1",
            0xDD2F => "PostalAddressOtherLine2",
            0xDD30 => "PostalAddressOtherCity",
            0xDD31 => "PostalAddressOtherRegion",
            0xDD32 => "PostalAddressOtherPostalCode",
            0xDD33 => "PostalAddressOtherCountry",
            0xDD34 => "OrganizationName",
            0xDD35 => "PhoneticOrganizationName",
            0xDD36 => "Role",
            0xDD37 => "Birthdate",
            0xDD40 => "MessageTo",
            0xDD41 => "MessageCC",
            0xDD42 => "MessageBCC",
            0xDD43 => "MessageRead",
            0xDD44 => "MessageReceivedTime",
            0xDD45 => "MessageSender",
            0xDD50 => "ActivityBeginTime",
            0xDD51 => "ActivityEndTime",
            0xDD52 => "ActivityLocation",
            0xDD54 => "ActivityRequiredAttendees",
            0xDD55 => "ActivityOptionalAttendees",
            0xDD56 => "ActivityResources",
            0xDD57 => "ActivityAccepted",
            0xDD5D => "Owner",
            0xDD5E => "Editor",
            0xDD5F => "Webmaster",
            0xDD60 => "URLSource",
            0xDD61 => "URLDestination",
            0xDD62 => "TimeBookmark",
            0xDD63 => "ObjectBookmark",
            0xDD64 => "ByteBookmark",
            0xDD70 => "LastBuildDate",
            0xDD71 => "TimetoLive",
            0xDD72 => "MediaGUID",
            0xDE91 => "TotalBitRate",
            0xDE92 => "BitRateType",
            0xDE93 => "SampleRate",
            0xDE94 => "NumberOfChannels",
            0xDE95 => "AudioBitDepth",
            0xDE97 => "ScanDepth",
            0xDE99 => "AudioWAVECodec",
            0xDE9A => "AudioBitRate",
            0xDE9B => "VideoFourCCCodec",
            0xDE9C => "VideoBitRate",
            0xDE9D => "FramesPerThousandSeconds",
            0xDE9E => "KeyFrameDistance",
            0xDE9F => "BufferSize",
            0xDEA0 => "EncodingQuality",
            0xDEA1 => "EncodingProfile",
            _ => return None,
        })
    }
}

impl fmt::Display for ObjectPropCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// data type code (`DTC_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct DataType(pub u16);

impl DataType {
    pub const UNDEF: Self = Self(0x0000);
    pub const INT8: Self = Self(0x0001);
    pub const UINT8: Self = Self(0x0002);
    pub const INT16: Self = Self(0x0003);
    pub const UINT16: Self = Self(0x0004);
    pub const INT32: Self = Self(0x0005);
    pub const UINT32: Self = Self(0x0006);
    pub const INT64: Self = Self(0x0007);
    pub const UINT64: Self = Self(0x0008);
    pub const INT128: Self = Self(0x0009);
    pub const UINT128: Self = Self(0x000A);
    pub const ARRAY_MASK: Self = Self(0x4000);
    pub const STR: Self = Self(0xFFFF);

    /// Debug name from go-mtpfs `DTC_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "UNDEF",
            0x0001 => "INT8",
            0x0002 => "UINT8",
            0x0003 => "INT16",
            0x0004 => "UINT16",
            0x0005 => "INT32",
            0x0006 => "UINT32",
            0x0007 => "INT64",
            0x0008 => "UINT64",
            0x0009 => "INT128",
            0x000A => "UINT128",
            0x4000 => "ARRAY_MASK",
            0xFFFF => "STR",
            _ => return None,
        })
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// storage type (`ST_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct StorageType(pub u16);

impl StorageType {
    pub const UNDEFINED: Self = Self(0x0000);
    pub const FIXED_ROM: Self = Self(0x0001);
    pub const REMOVABLE_ROM: Self = Self(0x0002);
    pub const FIXED_RAM: Self = Self(0x0003);
    pub const REMOVABLE_RAM: Self = Self(0x0004);

    /// Debug name from go-mtpfs `ST_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "Undefined",
            0x0001 => "FixedROM",
            0x0002 => "RemovableROM",
            0x0003 => "FixedRAM",
            0x0004 => "RemovableRAM",
            _ => return None,
        })
    }
}

impl fmt::Display for StorageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// filesystem type (`FST_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct FilesystemType(pub u16);

impl FilesystemType {
    pub const UNDEFINED: Self = Self(0x0000);
    pub const GENERIC_FLAT: Self = Self(0x0001);
    pub const GENERIC_HIERARCHICAL: Self = Self(0x0002);
    pub const DCF: Self = Self(0x0003);

    /// Debug name from go-mtpfs `FST_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "Undefined",
            0x0001 => "GenericFlat",
            0x0002 => "GenericHierarchical",
            0x0003 => "DCF",
            _ => return None,
        })
    }
}

impl fmt::Display for FilesystemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// storage access capability (`AC_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct AccessCapability(pub u16);

impl AccessCapability {
    pub const READ_WRITE: Self = Self(0x0000);
    pub const READ_ONLY: Self = Self(0x0001);
    pub const READ_ONLY_WITH_OBJECT_DELETION: Self = Self(0x0002);

    /// Debug name from go-mtpfs `AC_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "ReadWrite",
            0x0001 => "ReadOnly",
            0x0002 => "ReadOnly_with_Object_Deletion",
            _ => return None,
        })
    }
}

impl fmt::Display for AccessCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// association type (`AT_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct AssociationType(pub u16);

impl AssociationType {
    pub const UNDEFINED: Self = Self(0x0000);
    pub const GENERIC_FOLDER: Self = Self(0x0001);
    pub const ALBUM: Self = Self(0x0002);
    pub const TIME_SEQUENCE: Self = Self(0x0003);
    pub const HORIZONTAL_PANORAMIC: Self = Self(0x0004);
    pub const VERTICAL_PANORAMIC: Self = Self(0x0005);
    pub const PANORAMIC_2D: Self = Self(0x0006);
    pub const ANCILLARY_DATA: Self = Self(0x0007);

    /// Debug name from go-mtpfs `AT_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "Undefined",
            0x0001 => "GenericFolder",
            0x0002 => "Album",
            0x0003 => "TimeSequence",
            0x0004 => "HorizontalPanoramic",
            0x0005 => "VerticalPanoramic",
            0x0006 => "2DPanoramic",
            0x0007 => "AncillaryData",
            _ => return None,
        })
    }
}

impl fmt::Display for AssociationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// byte-order directionality (`DL_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Directionality(pub u16);

impl Directionality {
    pub const LE: Self = Self(0x000F);
    pub const BE: Self = Self(0x00F0);

    /// Debug name from go-mtpfs `DL_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x000F => "LE",
            0x00F0 => "BE",
            _ => return None,
        })
    }
}

impl fmt::Display for Directionality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// device property form field (`DPFF_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct DevicePropFormFlag(pub u16);

impl DevicePropFormFlag {
    pub const NONE: Self = Self(0x0000);
    pub const RANGE: Self = Self(0x0001);
    pub const ENUMERATION: Self = Self(0x0002);

    /// Debug name from go-mtpfs `DPFF_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "None",
            0x0001 => "Range",
            0x0002 => "Enumeration",
            _ => return None,
        })
    }
}

impl fmt::Display for DevicePropFormFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// device property get/set (`DPGS_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct DevicePropGetSet(pub u16);

impl DevicePropGetSet {
    pub const GET: Self = Self(0x0000);
    pub const GET_SET: Self = Self(0x0001);

    /// Debug name from go-mtpfs `DPGS_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "Get",
            0x0001 => "GetSet",
            _ => return None,
        })
    }
}

impl fmt::Display for DevicePropGetSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// PTP-layer error code (`ERROR_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PtpErrorCode(pub u16);

impl PtpErrorCode {
    pub const TIMEOUT: Self = Self(0x02FA);
    pub const CANCEL: Self = Self(0x02FB);
    pub const BADPARAM: Self = Self(0x02FC);
    pub const RESP_EXPECTED: Self = Self(0x02FD);
    pub const DATA_EXPECTED: Self = Self(0x02FE);
    pub const IO: Self = Self(0x02FF);

    /// Debug name from go-mtpfs `ERROR_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x02FA => "TIMEOUT",
            0x02FB => "CANCEL",
            0x02FC => "BADPARAM",
            0x02FD => "RESP_EXPECTED",
            0x02FE => "DATA_EXPECTED",
            0x02FF => "IO",
            _ => return None,
        })
    }
}

impl fmt::Display for PtpErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// Nikon vendor constant (`NIKON_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NikonConst(pub u16);

impl NikonConst {
    pub const MAX_CURVE_POINTS: Self = Self(0x0013);

    /// Debug name from go-mtpfs `NIKON_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0013 => "MaxCurvePoints",
            _ => return None,
        })
    }
}

impl fmt::Display for NikonConst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// object property form field (`OPFF_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ObjectPropFormFlag(pub u16);

impl ObjectPropFormFlag {
    pub const NONE: Self = Self(0x0000);
    pub const RANGE: Self = Self(0x0001);
    pub const ENUMERATION: Self = Self(0x0002);
    pub const DATE_TIME: Self = Self(0x0003);
    pub const FIXED_LENGTH_ARRAY: Self = Self(0x0004);
    pub const REGULAR_EXPRESSION: Self = Self(0x0005);
    pub const BYTE_ARRAY: Self = Self(0x0006);
    pub const LONG_STRING: Self = Self(0x00FF);

    /// Debug name from go-mtpfs `OPFF_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "None",
            0x0001 => "Range",
            0x0002 => "Enumeration",
            0x0003 => "DateTime",
            0x0004 => "FixedLengthArray",
            0x0005 => "RegularExpression",
            0x0006 => "ByteArray",
            0x00FF => "LongString",
            _ => return None,
        })
    }
}

impl fmt::Display for ObjectPropFormFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// protection status (`PS_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ProtectionStatus(pub u16);

impl ProtectionStatus {
    pub const NO_PROTECTION: Self = Self(0x0000);
    pub const READ_ONLY: Self = Self(0x0001);
    pub const MTP_READ_ONLY_DATA: Self = Self(0x8002);
    pub const MTP_NON_TRANSFERABLE_DATA: Self = Self(0x8003);

    /// Debug name from go-mtpfs `PS_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "NoProtection",
            0x0001 => "ReadOnly",
            0x8002 => "MTP_ReadOnlyData",
            0x8003 => "MTP_NonTransferableData",
            _ => return None,
        })
    }
}

impl fmt::Display for ProtectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// USB container / bulk constant (`USB_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct UsbContainer(pub u16);

impl UsbContainer {
    pub const CONTAINER_UNDEFINED: Self = Self(0x0000);
    pub const CONTAINER_COMMAND: Self = Self(0x0001);
    pub const CONTAINER_DATA: Self = Self(0x0002);
    pub const CONTAINER_RESPONSE: Self = Self(0x0003);
    pub const CONTAINER_EVENT: Self = Self(0x0004);
    pub const BULK_HS_MAX_PACKET_LEN_READ: Self = Self(0x0200);
    pub const BULK_HS_MAX_PACKET_LEN_WRITE: Self = Self(0x0200);

    /// Debug name from go-mtpfs `USB_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "CONTAINER_UNDEFINED",
            0x0001 => "CONTAINER_COMMAND",
            0x0002 => "CONTAINER_DATA",
            0x0003 => "CONTAINER_RESPONSE",
            0x0004 => "CONTAINER_EVENT",
            0x0200 => "BULK_HS_MAX_PACKET_LEN_READ",
            _ => return None,
        })
    }
}

impl fmt::Display for UsbContainer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// get object handles selector (`GOH_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct GetObjectHandles(pub u32);

impl GetObjectHandles {
    pub const ALL_ASSOCS: Self = Self(0x00000000);
    pub const ALL_FORMATS: Self = Self(0x00000000);
    pub const ALL_STORAGE: Self = Self(0xFFFFFFFF);
    pub const ROOT_PARENT: Self = Self(0xFFFFFFFF);

    /// Debug name from go-mtpfs `GOH_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x00000000 => "ALL_ASSOCS",
            0xFFFFFFFF => "ALL_STORAGE",
            _ => return None,
        })
    }
}

impl fmt::Display for GetObjectHandles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:08X}", self.0),
        }
    }
}

/// object handler selector (`HANDLER_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Handler(pub u32);

impl Handler {
    pub const ROOT: Self = Self(0x00000000);
    pub const SPECIAL: Self = Self(0xFFFFFFFF);

    /// Debug name from go-mtpfs `HANDLER_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x00000000 => "ROOT",
            0xFFFFFFFF => "SPECIAL",
            _ => return None,
        })
    }
}

impl fmt::Display for Handler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:08X}", self.0),
        }
    }
}

/// MTP/PTP vendor extension id (`VENDOR_*` in go-mtpfs const.go).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Vendor(pub u32);

impl Vendor {
    pub const EASTMAN_KODAK: Self = Self(0x00000001);
    pub const SEIKO_EPSON: Self = Self(0x00000002);
    pub const AGILENT: Self = Self(0x00000003);
    pub const POLAROID: Self = Self(0x00000004);
    pub const AGFA_GEVAERT: Self = Self(0x00000005);
    pub const MICROSOFT: Self = Self(0x00000006);
    pub const EQUINOX: Self = Self(0x00000007);
    pub const VIEWQUEST: Self = Self(0x00000008);
    pub const STMICROELECTRONICS: Self = Self(0x00000009);
    pub const NIKON: Self = Self(0x0000000A);
    pub const CANON: Self = Self(0x0000000B);
    pub const FOTONATION: Self = Self(0x0000000C);
    pub const PENTAX: Self = Self(0x0000000D);
    pub const FUJI: Self = Self(0x0000000E);

    /// Debug name from go-mtpfs `VENDOR_names` (const.go). `None` for
    /// values munge.py filtered out (vendor extensions / duplicate values).
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x00000001 => "EASTMAN_KODAK",
            0x00000002 => "SEIKO_EPSON",
            0x00000003 => "AGILENT",
            0x00000004 => "POLAROID",
            0x00000005 => "AGFA_GEVAERT",
            0x00000006 => "MICROSOFT",
            0x00000007 => "EQUINOX",
            0x00000008 => "VIEWQUEST",
            0x00000009 => "STMICROELECTRONICS",
            0x0000000A => "NIKON",
            0x0000000B => "CANON",
            0x0000000C => "FOTONATION",
            0x0000000D => "PENTAX",
            0x0000000E => "FUJI",
            _ => return None,
        })
    }
}

impl fmt::Display for Vendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "0x{:08X}", self.0),
        }
    }
}

/// Selects which constant group [`code_name`] resolves a `u16` in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CodeKind {
    Op,
    Resp,
    Event,
    ObjectFormat,
    DeviceProp,
    ObjectProp,
    DataType,
    StorageType,
    FilesystemType,
    AccessCapability,
    AssociationType,
    Directionality,
    DevicePropFormFlag,
    DevicePropGetSet,
    PtpError,
    Nikon,
    ObjectPropFormFlag,
    ProtectionStatus,
    Usb,
}

/// Debug lookup mirroring go-mtpfs `PREFIX_names[code]` for the `u16` groups.
///
/// Returns the spec name for `code` within `kind`, or `None` when the value is
/// absent from that group's Go name map (vendor extension / duplicate value).
/// The `u32` selector groups (GOH/HANDLER/VENDOR) resolve via their own
/// `name()` methods, not here (their sentinels don't fit `u16`).
pub fn code_name(kind: CodeKind, code: u16) -> Option<&'static str> {
    match kind {
        CodeKind::Op => OpCode(code).name(),
        CodeKind::Resp => RespCode(code).name(),
        CodeKind::Event => EventCode(code).name(),
        CodeKind::ObjectFormat => ObjectFormat(code).name(),
        CodeKind::DeviceProp => DevicePropCode(code).name(),
        CodeKind::ObjectProp => ObjectPropCode(code).name(),
        CodeKind::DataType => DataType(code).name(),
        CodeKind::StorageType => StorageType(code).name(),
        CodeKind::FilesystemType => FilesystemType(code).name(),
        CodeKind::AccessCapability => AccessCapability(code).name(),
        CodeKind::AssociationType => AssociationType(code).name(),
        CodeKind::Directionality => Directionality(code).name(),
        CodeKind::DevicePropFormFlag => DevicePropFormFlag(code).name(),
        CodeKind::DevicePropGetSet => DevicePropGetSet(code).name(),
        CodeKind::PtpError => PtpErrorCode(code).name(),
        CodeKind::Nikon => NikonConst(code).name(),
        CodeKind::ObjectPropFormFlag => ObjectPropFormFlag(code).name(),
        CodeKind::ProtectionStatus => ProtectionStatus(code).name(),
        CodeKind::Usb => UsbContainer(code).name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 25 known values spanning every constant group. Values are load-bearing
    // spec facts (go-mtpfs const.go); a mismatch here is a wire-protocol bug.
    #[test]
    fn spot_check_values() {
        // operation codes
        assert_eq!(OpCode::GET_DEVICE_INFO, OpCode(0x1001));
        assert_eq!(OpCode::OPEN_SESSION, OpCode(0x1002));
        assert_eq!(OpCode::SEND_OBJECT, OpCode(0x100D));
        assert_eq!(OpCode::GET_STORAGE_IDS, OpCode(0x1004));
        assert_eq!(OpCode::MTP_GET_OBJECT_PROP_VALUE, OpCode(0x9803));
        // android opcodes (android.go)
        assert_eq!(OpCode::ANDROID_GET_PARTIAL_OBJECT64, OpCode(0x95C1));
        assert_eq!(OpCode::ANDROID_SEND_PARTIAL_OBJECT, OpCode(0x95C2));
        assert_eq!(OpCode::ANDROID_TRUNCATE_OBJECT, OpCode(0x95C3));
        assert_eq!(OpCode::ANDROID_BEGIN_EDIT_OBJECT, OpCode(0x95C4));
        assert_eq!(OpCode::ANDROID_END_EDIT_OBJECT, OpCode(0x95C5));
        // return codes
        assert_eq!(RespCode::OK, RespCode(0x2001));
        assert_eq!(RespCode::GENERAL_ERROR, RespCode(0x2002));
        assert_eq!(RespCode::INVALID_OBJECT_HANDLE, RespCode(0x2009));
        assert_eq!(RespCode::SESSION_ALREADY_OPENED, RespCode(0x201E));
        // event codes
        assert_eq!(EventCode::OBJECT_ADDED, EventCode(0x4002));
        assert_eq!(EventCode::STORE_FULL, EventCode(0x400A));
        // object formats
        assert_eq!(ObjectFormat::ASSOCIATION, ObjectFormat(0x3001));
        assert_eq!(ObjectFormat::MTP_MP4, ObjectFormat(0xB982));
        // object property codes
        assert_eq!(ObjectPropCode::OBJECT_FILE_NAME, ObjectPropCode(0xDC07));
        assert_eq!(ObjectPropCode::OBJECT_SIZE, ObjectPropCode(0xDC04));
        assert_eq!(ObjectPropCode::STORAGE_ID, ObjectPropCode(0xDC01));
        // data type codes
        assert_eq!(DataType::UINT128, DataType(0x000A));
        assert_eq!(DataType::INT128, DataType(0x0009));
        assert_eq!(DataType::STR, DataType(0xFFFF));
        assert_eq!(DataType::ARRAY_MASK, DataType(0x4000));
        // device property, storage, filesystem, access capability
        assert_eq!(DevicePropCode::BATTERY_LEVEL, DevicePropCode(0x5001));
        assert_eq!(StorageType::REMOVABLE_RAM, StorageType(0x0004));
        assert_eq!(FilesystemType::DCF, FilesystemType(0x0003));
        assert_eq!(AccessCapability::READ_ONLY, AccessCapability(0x0001));
        assert_eq!(
            AccessCapability::READ_ONLY_WITH_OBJECT_DELETION,
            AccessCapability(0x0002)
        );
    }

    // The two documented ident deviations still carry the exact Go values.
    #[test]
    fn documented_deviations_preserve_values() {
        assert_eq!(AssociationType::PANORAMIC_2D, AssociationType(0x0006));
        assert_eq!(DevicePropCode::NIKON_ISO_AUTO, DevicePropCode(0xD054));
        assert_eq!(DevicePropCode::NIKON_ISO_AUTO_ALT, DevicePropCode(0xD16A));
    }

    #[test]
    fn code_name_lookup_matches_go_maps() {
        assert_eq!(code_name(CodeKind::Op, 0x1001), Some("GetDeviceInfo"));
        assert_eq!(
            code_name(CodeKind::Resp, 0x201E),
            Some("SessionAlreadyOpened")
        );
        assert_eq!(
            code_name(CodeKind::ObjectFormat, 0x3001),
            Some("Association")
        );
        assert_eq!(
            code_name(CodeKind::ObjectProp, 0xDC07),
            Some("ObjectFileName")
        );
        assert_eq!(code_name(CodeKind::DataType, 0x000A), Some("UINT128"));
        // android additions land in OC_names via android.go init()
        assert_eq!(
            code_name(CodeKind::Op, 0x95C1),
            Some("ANDROID_GET_PARTIAL_OBJECT64")
        );
        // vendor extensions were filtered out of the Go name maps -> None.
        assert_eq!(code_name(CodeKind::Op, 0x9001), None); // CANON_GetPartialObjectInfo
        assert_eq!(code_name(CodeKind::Op, 0xFFFF), None);
    }

    #[test]
    fn display_uses_spec_name_else_hex() {
        assert_eq!(OpCode::GET_DEVICE_INFO.to_string(), "GetDeviceInfo");
        assert_eq!(RespCode::OK.to_string(), "OK");
        // unknown code falls back to hex
        assert_eq!(OpCode(0x9001).to_string(), "0x9001");
    }
}
