import Foundation

public enum AddressListKeyKind: Equatable, Sendable {
    case privateKey
    case publicKey
}

public struct AddressListParseResult: Equatable, Sendable {
    public let entries: [String]
    public let valid: [String]
    public let invalid: [String]
    public let duplicates: Int
    public let privateKeys: Int
    public let publicKeys: Int

    public init(
        entries: [String], valid: [String], invalid: [String], duplicates: Int,
        privateKeys: Int, publicKeys: Int
    ) {
        self.entries = entries
        self.valid = valid
        self.invalid = invalid
        self.duplicates = duplicates
        self.privateKeys = privateKeys
        self.publicKeys = publicKeys
    }
}

public struct AddressListScrubResult: Equatable, Sendable {
    public let text: String
    public let privateKeys: Int
    public let publicKeys: Int
}

/// Native counterpart of `ui-tauri/src/lib/addressList.ts`. The parser is
/// intentionally stricter than the daemon's normalizer so a watch-only address
/// list never retains private or extended-public key material in view state.
public enum AddressListParser {
    private static let base58Alphabet = Array("123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz")
    private static let base58Index = Dictionary(uniqueKeysWithValues: base58Alphabet.enumerated().map { ($0.element, $0.offset) })
    private static let bech32Charset = Array("qpzry9x8gf2tvdw0s3jn54khce6mua7l")
    private static let bech32Index = Dictionary(uniqueKeysWithValues: bech32Charset.enumerated().map { ($0.element, $0.offset) })

    public static func classifyKeyMaterial(_ value: String) -> AddressListKeyKind? {
        let token = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if matches(token, #"^[59KLc][1-9A-HJ-NP-Za-km-z]{50,51}$"#)
            || matches(token, #"^6P[1-9A-HJ-NP-Za-km-z]{56}$"#)
            || matches(token, #"^(?:xprv|yprv|zprv|tprv|uprv|vprv|Yprv|Zprv|Uprv|Vprv)[1-9A-HJ-NP-Za-km-z]{70,120}$"#) {
            return .privateKey
        }
        if matches(token, #"^(?:xpub|ypub|zpub|tpub|upub|vpub|Ypub|Zpub|Upub|Vpub)[1-9A-HJ-NP-Za-km-z]{70,120}$"#)
            || matches(token, #"^(?:0[23][0-9a-fA-F]{64}|04[0-9a-fA-F]{128})$"#) {
            return .publicKey
        }
        return nil
    }

    public static func looksLikeMainnetAddress(_ value: String) -> Bool {
        let token = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !token.isEmpty else { return false }
        if let payload = base58CheckPayload(token) {
            return payload.count == 21 && (payload[0] == 0x00 || payload[0] == 0x05)
        }
        guard let decoded = bech32Decode(token), decoded.hrp == "bc", !decoded.data.isEmpty,
              let version = decoded.data.first, version <= 16,
              let program = convertBits(Array(decoded.data.dropFirst()), fromBits: 5, toBits: 8, pad: false),
              (2...40).contains(program.count) else { return false }
        if version == 0 {
            return decoded.spec == .bech32 && (program.count == 20 || program.count == 32)
        }
        return decoded.spec == .bech32m
    }

    public static func parse(_ input: String) -> AddressListParseResult {
        var entries: [String] = []
        var valid: [String] = []
        var invalid: [String] = []
        var seenValid = Set<String>()
        var seenInvalid = Set<String>()
        var duplicates = 0
        var privateKeys = 0
        var publicKeys = 0
        for token in tokens(input) {
            switch classifyKeyMaterial(token) {
            case .privateKey?: privateKeys += 1; continue
            case .publicKey?: publicKeys += 1; continue
            case nil: break
            }
            entries.append(token)
            if looksLikeMainnetAddress(token) {
                if !seenValid.insert(token).inserted { duplicates += 1 }
                else { valid.append(token) }
            } else if seenInvalid.insert(token).inserted {
                invalid.append(token)
            }
        }
        return AddressListParseResult(
            entries: entries, valid: valid, invalid: invalid, duplicates: duplicates,
            privateKeys: privateKeys, publicKeys: publicKeys
        )
    }

    public static func scrubKeyMaterial(_ input: String) -> AddressListScrubResult {
        var survivors: [String] = []
        var privateKeys = 0
        var publicKeys = 0
        for token in tokens(input) {
            switch classifyKeyMaterial(token) {
            case .privateKey?: privateKeys += 1
            case .publicKey?: publicKeys += 1
            case nil: survivors.append(token)
            }
        }
        return AddressListScrubResult(
            text: survivors.joined(separator: "\n"), privateKeys: privateKeys, publicKeys: publicKeys
        )
    }

    private static func tokens(_ input: String) -> [String] {
        let separators = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: ",;"))
        return input.components(separatedBy: separators).filter { !$0.isEmpty }
    }

    private static func matches(_ value: String, _ pattern: String) -> Bool {
        value.range(of: pattern, options: .regularExpression) != nil
    }

    private static func base58CheckPayload(_ value: String) -> [UInt8]? {
        var bytes = [UInt8](repeating: 0, count: value.count)
        var length = 0
        for character in value {
            guard let digit = base58Index[character] else { return nil }
            var carry = digit
            var index = 0
            while index < length {
                let position = bytes.count - 1 - index
                carry += Int(bytes[position]) * 58
                bytes[position] = UInt8(carry & 0xff)
                carry >>= 8
                index += 1
            }
            while carry > 0 {
                length += 1
                bytes[bytes.count - length] = UInt8(carry & 0xff)
                carry >>= 8
            }
        }
        let leadingZeros = value.prefix(while: { $0 == "1" }).count
        let significant = Array(bytes.suffix(length))
        let decoded = [UInt8](repeating: 0, count: leadingZeros) + significant
        guard decoded.count >= 5 else { return nil }
        let body = Array(decoded.dropLast(4))
        let checksum = Array(decoded.suffix(4))
        return Array(sha256(sha256(body)).prefix(4)) == checksum ? body : nil
    }

    private enum Bech32Spec { case bech32, bech32m }
    private static func bech32Decode(_ value: String) -> (hrp: String, data: [Int], spec: Bech32Spec)? {
        guard value == value.lowercased() || value == value.uppercased() else { return nil }
        let normalized = value.lowercased()
        guard let separator = normalized.lastIndex(of: "1") else { return nil }
        let offset = normalized.distance(from: normalized.startIndex, to: separator)
        guard offset >= 1, offset + 7 <= normalized.count else { return nil }
        let hrp = String(normalized[..<separator])
        var data: [Int] = []
        for character in normalized[normalized.index(after: separator)...] {
            guard let number = bech32Index[character] else { return nil }
            data.append(number)
        }
        let polymod = bech32Polymod(bech32HrpExpand(hrp) + data)
        let spec: Bech32Spec
        if polymod == 1 { spec = .bech32 }
        else if polymod == 0x2bc830a3 { spec = .bech32m }
        else { return nil }
        return (hrp, Array(data.dropLast(6)), spec)
    }

    private static func bech32HrpExpand(_ hrp: String) -> [Int] {
        hrp.unicodeScalars.map { Int($0.value) >> 5 } + [0]
            + hrp.unicodeScalars.map { Int($0.value) & 31 }
    }

    private static func bech32Polymod(_ values: [Int]) -> Int {
        let generators = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
        var checksum = 1
        for value in values {
            let top = checksum >> 25
            checksum = ((checksum & 0x1ffffff) << 5) ^ value
            for index in generators.indices where ((top >> index) & 1) != 0 { checksum ^= generators[index] }
        }
        return checksum
    }

    private static func convertBits(_ data: [Int], fromBits: Int, toBits: Int, pad: Bool) -> [Int]? {
        var accumulator = 0
        var bits = 0
        var output: [Int] = []
        let maxValue = (1 << toBits) - 1
        let maxAccumulator = (1 << (fromBits + toBits - 1)) - 1
        for value in data {
            guard value >= 0, (value >> fromBits) == 0 else { return nil }
            accumulator = ((accumulator << fromBits) | value) & maxAccumulator
            bits += fromBits
            while bits >= toBits {
                bits -= toBits
                output.append((accumulator >> bits) & maxValue)
            }
        }
        if pad, bits > 0 { output.append((accumulator << (toBits - bits)) & maxValue) }
        else if !pad && (bits >= fromBits || ((accumulator << (toBits - bits)) & maxValue) != 0) { return nil }
        return output
    }

    // Compact SHA-256 used only for Base58Check validation. This keeps the
    // ViewModels target Foundation-only as required by the native architecture.
    private static let sha256Constants: [UInt32] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ]

    private static func sha256(_ input: [UInt8]) -> [UInt8] {
        var message = input
        let bitLength = UInt64(message.count) * 8
        message.append(0x80)
        while message.count % 64 != 56 { message.append(0) }
        for shift in stride(from: 56, through: 0, by: -8) { message.append(UInt8((bitLength >> UInt64(shift)) & 0xff)) }
        var hash: [UInt32] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]
        var words = [UInt32](repeating: 0, count: 64)
        for offset in stride(from: 0, to: message.count, by: 64) {
            for index in 0..<16 {
                let base = offset + index * 4
                words[index] = (UInt32(message[base]) << 24) | (UInt32(message[base + 1]) << 16)
                    | (UInt32(message[base + 2]) << 8) | UInt32(message[base + 3])
            }
            for index in 16..<64 {
                let s0 = rotateRight(words[index - 15], 7) ^ rotateRight(words[index - 15], 18) ^ (words[index - 15] >> 3)
                let s1 = rotateRight(words[index - 2], 17) ^ rotateRight(words[index - 2], 19) ^ (words[index - 2] >> 10)
                words[index] = words[index - 16] &+ s0 &+ words[index - 7] &+ s1
            }
            var work = hash
            for index in 0..<64 {
                let s1 = rotateRight(work[4], 6) ^ rotateRight(work[4], 11) ^ rotateRight(work[4], 25)
                let choose = (work[4] & work[5]) ^ (~work[4] & work[6])
                let temp1 = work[7] &+ s1 &+ choose &+ sha256Constants[index] &+ words[index]
                let s0 = rotateRight(work[0], 2) ^ rotateRight(work[0], 13) ^ rotateRight(work[0], 22)
                let majority = (work[0] & work[1]) ^ (work[0] & work[2]) ^ (work[1] & work[2])
                let temp2 = s0 &+ majority
                work = [temp1 &+ temp2, work[0], work[1], work[2], work[3] &+ temp1, work[4], work[5], work[6]]
            }
            for index in hash.indices { hash[index] &+= work[index] }
        }
        return hash.flatMap { word in [UInt8(word >> 24), UInt8((word >> 16) & 0xff), UInt8((word >> 8) & 0xff), UInt8(word & 0xff)] }
    }

    private static func rotateRight(_ value: UInt32, _ bits: UInt32) -> UInt32 {
        (value >> bits) | (value << (32 - bits))
    }
}
