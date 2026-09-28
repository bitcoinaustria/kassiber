import { describe, expect, it } from "vitest";

import { noteActiveImportProject } from "@/daemon/transport";

import { regtestDemoImportedProject } from "./regtestDemo";

const demo = {
  stateRoot: "/books/demo",
  dataRoot: "/books/demo/data",
  database: "/books/demo/data/kassiber.sqlite3",
};
const status = {
  state_root: demo.stateRoot,
  data_root: demo.dataRoot,
  database: demo.database,
  current_workspace: "Regtest Demo",
};

// These cases share the transport's module-level import selection, so they
// run in order: nothing picked, another folder picked, this folder picked.
describe("regtestDemoImportedProject", () => {
  it("leaves the daemon's launch book unimported", () => {
    expect(regtestDemoImportedProject(status)).toBeUndefined();
  });

  it("ignores an import of a different folder", () => {
    noteActiveImportProject({
      stateRoot: "/books/other",
      dataRoot: "/books/other/data",
      database: "/books/other/data/kassiber.sqlite3",
      encrypted: false,
    });
    expect(regtestDemoImportedProject(status)).toBeUndefined();
  });

  it("keeps a folder the user picked as the imported project", () => {
    noteActiveImportProject({ ...demo, encrypted: false });
    expect(regtestDemoImportedProject(status)).toEqual(demo);
    expect(
      regtestDemoImportedProject({ ...status, database: null }),
    ).toBeUndefined();
  });
});
