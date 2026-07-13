import Foundation

// MARK: - Envelope
//
// Every keel payload is `{"errorType": "", "error": "", "data": ...}` —
// empty strings mean success (mirrors `_getData` in the original bindings).

public struct KeelEnvelope<T: Decodable & Sendable>: Decodable, Sendable {
    public let errorType: String?
    public let error: String?
    public let data: T?

    public var failure: KeelError? {
        guard let error, !error.isEmpty else { return nil }
        return KeelError.kernel(type: errorType ?? "", message: error)
    }
}

public enum KeelError: Error, CustomStringConvertible, Sendable {
    /// Error reported by the keel kernel (e.g. MtpDetectFailed).
    case kernel(type: String, message: String)
    /// The callback payload wasn't valid JSON.
    case malformedPayload(String)
    /// An operation was attempted before initialize() succeeded.
    case notConnected

    public var description: String {
        switch self {
        case .kernel(let type, let message):
            return type.isEmpty ? message : "\(type): \(message)"
        case .malformedPayload(let raw):
            return "malformed keel payload: \(raw.prefix(200))"
        case .notConnected:
            return "no MTP device session — call initialize() first"
        }
    }

    public var kernelType: String? {
        if case .kernel(let type, _) = self { return type }
        return nil
    }

    /// True when the kernel simply couldn't find a phone — the expected
    /// idle state, not a fault.
    public var isDeviceNotFound: Bool {
        guard let kernelType else { return false }
        return kernelType.localizedCaseInsensitiveContains("detect")
            || kernelType.localizedCaseInsensitiveContains("notfound")
            || kernelType.localizedCaseInsensitiveContains("changed")
    }

    /// USB setup died mid-handshake (`ErrorDeviceSetup`) — the phone reset
    /// and re-enumerated with a new bus address, or another MTP client holds
    /// the interface. Usually transient: dispose and retry once the bus
    /// settles. (Samsungs are the classic case.)
    public var isDeviceSetupFailure: Bool {
        kernelType == "ErrorDeviceSetup"
    }

    /// The user cancelled the transfer (Ferry kernel extension) — an outcome,
    /// not a fault.
    public var isCancellation: Bool {
        kernelType == "ErrorTransferCancelled"
    }
}

// MARK: - Device

/// `data` of Initialize / FetchDeviceInfo. The nested mtp/usb structs come
/// from go-mtpfs without JSON tags, so keys are Go field names; decode the
/// handful we display and keep the rest as raw JSON for diagnostics.
public struct KeelDeviceInfo: Decodable, Sendable {
    public let mtpDeviceInfo: MTPDeviceInfo?
    public let usbDeviceInfo: USBDeviceInfo?

    public struct MTPDeviceInfo: Decodable, Sendable {
        public let manufacturer: String?
        public let model: String?
        public let deviceVersion: String?
        public let serialNumber: String?
        public let mtpExtension: String?

        enum CodingKeys: String, CodingKey {
            case manufacturer = "Manufacturer"
            case model = "Model"
            case deviceVersion = "DeviceVersion"
            case serialNumber = "SerialNumber"
            case mtpExtension = "MTPExtension"
        }
    }

    public struct USBDeviceInfo: Decodable, Sendable {
        private let raw: [String: KeelJSONValue]

        public init(from decoder: Decoder) throws {
            raw = try decoder.singleValueContainer().decode([String: KeelJSONValue].self)
        }

        public subscript(key: String) -> KeelJSONValue? { raw[key] }
        public var keys: [String] { raw.keys.sorted() }
    }

    public var displayName: String {
        let m = mtpDeviceInfo?.manufacturer ?? ""
        let model = mtpDeviceInfo?.model ?? ""
        let name = "\(m) \(model)".trimmingCharacters(in: .whitespaces)
        return name.isEmpty ? "Android Device" : name
    }
}

// MARK: - Storage

/// `data` of FetchStorages: `[{"Sid": 65537, "Info": {...}}]`.
public struct KeelStorage: Decodable, Sendable, Identifiable, Hashable {
    public let sid: UInt32
    private let info: [String: KeelJSONValue]

    public var id: UInt32 { sid }

    enum CodingKeys: String, CodingKey {
        case sid = "Sid"
        case info = "Info"
    }

    public var description: String {
        info["StorageDescription"]?.stringValue
            ?? info["VolumeLabel"]?.stringValue
            ?? "Storage \(sid)"
    }

    public var maxCapacity: Int64? {
        info["MaxCapability"]?.int64Value ?? info["MaxCapacity"]?.int64Value
    }

    public var freeSpace: Int64? {
        info["FreeSpaceInBytes"]?.int64Value
    }
}

// MARK: - Files

/// `data` of Walk — matches `FileInfo` in send_to_js/structs.go exactly.
/// Codable (not just Decodable) so files can ride drag-and-drop as
/// Transferable payloads.
public struct KeelFile: Codable, Sendable, Identifiable, Hashable {
    public let size: Int64
    public let isFolder: Bool
    public let dateAdded: String
    public let name: String
    public let path: String
    public let parentPath: String
    public let `extension`: String
    public let parentId: UInt32
    public let objectId: UInt32

    public var id: UInt32 { objectId }

    enum CodingKeys: String, CodingKey {
        case size, isFolder, dateAdded, name, path, parentPath
        case `extension` = "extension"
        case parentId, objectId
    }
}

// MARK: - Transfers

public struct KeelTransferPreprocess: Decodable, Sendable {
    public let fullPath: String
    public let name: String
    public let size: Int64
}

public struct KeelTransferProgress: Decodable, Sendable {
    public struct SizeInfo: Decodable, Sendable {
        public let total: Int64
        public let sent: Int64
        public let progress: Float
    }

    public let fullPath: String
    public let name: String
    public let elapsedTime: Int64
    /// MB/s
    public let speed: Double
    public let totalFiles: Int64
    public let totalDirectories: Int64
    public let filesSent: Int64
    public let filesSentProgress: Float
    public let activeFileSize: SizeInfo
    public let bulkFileSize: SizeInfo
}

public enum KeelTransferEvent: Sendable {
    case preprocess(KeelTransferPreprocess)
    case progress(KeelTransferProgress)
    case completed
}

// MARK: - Flexible JSON
//
// Resilient container for Go structs whose exact shape we don't control
// (usb descriptors, storage info across device quirks).

public enum KeelJSONValue: Decodable, Sendable, Hashable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: KeelJSONValue])
    case array([KeelJSONValue])
    case null

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let b = try? container.decode(Bool.self) {
            self = .bool(b)
        } else if let n = try? container.decode(Double.self) {
            self = .number(n)
        } else if let s = try? container.decode(String.self) {
            self = .string(s)
        } else if let o = try? container.decode([String: KeelJSONValue].self) {
            self = .object(o)
        } else if let a = try? container.decode([KeelJSONValue].self) {
            self = .array(a)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container, debugDescription: "unsupported JSON value")
        }
    }

    public var stringValue: String? {
        if case .string(let s) = self, !s.isEmpty { return s }
        return nil
    }

    public var int64Value: Int64? {
        if case .number(let n) = self { return Int64(n) }
        return nil
    }
}
