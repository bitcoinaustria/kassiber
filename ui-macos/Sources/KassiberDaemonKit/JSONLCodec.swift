import Foundation

public enum JSONLCodec {
    private static let encoder = JSONEncoder()
    private static let decoder = JSONDecoder()

    public static func encode(_ request: DaemonRequest) throws -> Data {
        var data = try encoder.encode(request)
        data.append(0x0A)
        return data
    }

    public static func encode(_ value: JSONValue) throws -> Data {
        var data = try encoder.encode(value)
        data.append(0x0A)
        return data
    }

    public static func decodeRecord(_ data: Data) throws -> DaemonRecord {
        let trimmed = data.drop(while: { $0 == 0x20 || $0 == 0x09 || $0 == 0x0A || $0 == 0x0D })
        guard !trimmed.isEmpty else {
            throw DaemonClientError.protocolError("empty JSONL record")
        }
        return try decoder.decode(DaemonRecord.self, from: Data(trimmed))
    }
}

/// Incremental newline framing for FileHandle chunks. The daemon contract is
/// one UTF-8 JSON object per line; no record may span a logical newline.
public struct JSONLFramer: Sendable {
    private var buffer = Data()

    public init() {}

    public mutating func append(_ data: Data) -> [Data] {
        buffer.append(data)
        var records: [Data] = []
        while let newline = buffer.firstIndex(of: 0x0A) {
            let line = Data(buffer[..<newline])
            buffer.removeSubrange(...newline)
            if !line.isEmpty { records.append(line) }
        }
        return records
    }

    public mutating func finish() -> Data? {
        defer { buffer.removeAll(keepingCapacity: false) }
        return buffer.isEmpty ? nil : buffer
    }
}
