import { describe, expect, it } from "vitest";

import { endpointHost, endpointReach } from "./networkReach";

describe("networkReach", () => {
  it("names the host and port of any endpoint scheme", () => {
    expect(endpointHost("ssl://index.bitcoin-austria.at:50002")).toBe(
      "index.bitcoin-austria.at:50002",
    );
    expect(endpointHost("https://mempool.bitcoin-austria.at/api")).toBe(
      "mempool.bitcoin-austria.at",
    );
    expect(endpointHost("http://localhost:11434/v1")).toBe("localhost:11434");
  });


  it("keeps loopback endpoints on this device and everything else out", () => {
    expect(endpointReach("http://localhost:11434/v1")).toBe("device");
    expect(endpointReach("http://127.0.0.1:8332")).toBe("device");
    expect(endpointReach("http://[::1]:8332")).toBe("device");
    expect(endpointReach("http://[::ffff:127.0.0.1]:11434/v1")).toBe("device");
    expect(endpointReach("ssl://[::ffff:7f00:1]:50002")).toBe("device");
    expect(endpointReach("tcp://127.0.0.1:50001")).toBe("device");
    expect(endpointReach("localhost:50001")).toBe("device");
    expect(endpointReach("ssl://les.bullbitcoin.com:995")).toBe("internet");
    expect(endpointReach("http://abc.onion")).toBe("internet");
  });
});
