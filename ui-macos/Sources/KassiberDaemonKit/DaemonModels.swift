import Foundation

public struct DaemonRequest: Codable, Equatable, Sendable {
    public let kind: DaemonKind
    public let requestID: String
    public let args: [String: JSONValue]?

    enum CodingKeys: String, CodingKey {
        case kind
        case requestID = "request_id"
        case args
    }

    public init(kind: DaemonKind, requestID: String, args: [String: JSONValue]? = nil) {
        self.kind = kind
        self.requestID = requestID
        self.args = args
    }
}

public struct DaemonErrorPayload: Codable, Equatable, Sendable, Error {
    public let code: String
    public let message: String
    public let hint: String?
    public let details: JSONValue?
    public let retryable: Bool?
    public let debug: String?

    public init(
        code: String,
        message: String,
        hint: String? = nil,
        details: JSONValue? = nil,
        retryable: Bool? = nil,
        debug: String? = nil
    ) {
        self.code = code
        self.message = message
        self.hint = hint
        self.details = details
        self.retryable = retryable
        self.debug = debug
    }
}

public struct DaemonRecord: Codable, Equatable, Sendable {
    public let kind: String
    public let schemaVersion: Int
    public let requestID: JSONValue?
    public let event: Bool?
    public let data: JSONValue?
    public let error: DaemonErrorPayload?

    enum CodingKeys: String, CodingKey {
        case kind
        case schemaVersion = "schema_version"
        case requestID = "request_id"
        case event
        case data
        case error
    }

    public init(
        kind: String,
        schemaVersion: Int = 1,
        requestID: JSONValue? = nil,
        event: Bool? = nil,
        data: JSONValue? = nil,
        error: DaemonErrorPayload? = nil
    ) {
        self.kind = kind
        self.schemaVersion = schemaVersion
        self.requestID = requestID
        self.event = event
        self.data = data
        self.error = error
    }

    public var requestIDString: String? {
        switch requestID {
        case let .string(value): value
        case let .integer(value): String(value)
        case let .unsignedInteger(value): String(value)
        default: nil
        }
    }
}

public typealias DaemonEnvelope = DaemonRecord

public enum DaemonClientError: Error, Equatable, Sendable {
    case kindNotAllowed(String)
    case daemonNotReady(String)
    case daemonExited(String)
    case protocolError(String)
    case requestConflict(String)
    case requestTimedOut(kind: String, retryable: Bool)
    case transport(String)
}
