import Foundation
import Testing
@testable import KassiberDaemonKit

@Suite("JSONL protocol")
struct JSONLCodecTests {
    @Test("request encoding preserves exact integer args")
    func requestEncoding() throws {
        let request = DaemonRequest(
            kind: .uiTransactionsList,
            requestID: "macos-42",
            args: ["limit": .integer(25), "cursor": .null]
        )
        let data = try JSONLCodec.encode(request)
        #expect(data.last == 0x0A)
        let object = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        #expect(object?["kind"] as? String == "ui.transactions.list")
        #expect(object?["request_id"] as? String == "macos-42")
        #expect((object?["args"] as? [String: Any])?["limit"] as? Int == 25)
    }

    @Test("framer handles split and coalesced records")
    func framing() {
        var framer = JSONLFramer()
        #expect(framer.append(Data("{\"a\":".utf8)).isEmpty)
        let records = framer.append(Data("1}\n{\"b\":2}\npartial".utf8))
        #expect(records.map { String(decoding: $0, as: UTF8.self) } == ["{\"a\":1}", "{\"b\":2}"])
        #expect(String(decoding: framer.finish() ?? Data(), as: UTF8.self) == "partial")
    }

    @Test("record decoding accepts string and numeric request ids")
    func requestIDs() throws {
        let stringRecord = try JSONLCodec.decodeRecord(Data(#"{"kind":"x","schema_version":1,"request_id":"a"}"#.utf8))
        let numericRecord = try JSONLCodec.decodeRecord(Data(#"{"kind":"x","schema_version":1,"request_id":7}"#.utf8))
        #expect(stringRecord.requestIDString == "a")
        #expect(numericRecord.requestIDString == "7")
    }
}
