import { Vector3 } from "three";
import { describe, expect, it } from "vitest";

import { ribbonGeometry } from "./glassScene";

describe("glass ribbon mesh", () => {
  it("faces every triangle outward, sides and end caps alike", () => {
    // A straight ribbon along +x: every face normal must point away from its axis.
    const geometry = ribbonGeometry([0, 0], [4, 0]);
    const position = geometry.getAttribute("position");
    const index = geometry.getIndex()!;
    const corner = (at: number) => new Vector3().fromBufferAttribute(position, index.getX(at));
    let checked = 0;
    for (let at = 0; at < index.count; at += 3) {
      const [a, b, c] = [corner(at), corner(at + 1), corner(at + 2)];
      const normal = new Vector3().subVectors(b, a).cross(new Vector3().subVectors(c, a));
      if (normal.lengthSq() < 1e-12) continue;
      const centroid = a.clone().add(b).add(c).divideScalar(3);
      const outward = centroid.x <= 1e-6 ? new Vector3(-1, 0, 0) : centroid.x >= 4 - 1e-6
        ? new Vector3(1, 0, 0)
        : new Vector3(0, centroid.y, centroid.z);
      expect(normal.dot(outward)).toBeGreaterThan(0);
      checked += 1;
    }
    expect(checked).toBeGreaterThan(100);
  });
});
