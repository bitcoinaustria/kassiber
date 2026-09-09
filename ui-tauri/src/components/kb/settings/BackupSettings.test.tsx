import { isValidElement, type ReactNode } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { BackupSettings } from "./BackupSettings";

const state = vi.hoisted(() => ({ values: [] as unknown[], cursor: 0, refs: [] as {current: unknown}[], refCursor: 0, invoke: vi.fn(), pick: vi.fn(), save: vi.fn(), session: 1, notify: vi.fn() }));
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useEffect: () => {},
  useRef: (value: unknown) => state.refs[state.refCursor++] ?? (state.refs[state.refCursor - 1] = {current: value}),
  useState: (initial: unknown) => {
    const index = state.cursor++;
    if (!(index in state.values)) state.values[index] = initial;
    return [state.values[index], (next: unknown) => { state.values[index] = next; }];
  },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/daemon/transport", () => ({ getTransport: () => ({ invoke: state.invoke }) }));
vi.mock("@/daemon/client", () => ({ formatDaemonEnvelopeError: (result: {error?: {message: string}}) => result.error?.message }));
vi.mock("@/lib/filePicker", () => ({ isFilePickerAvailable: true, isFileSaveAvailable: true, pickFile: state.pick, saveFile: state.save }));
vi.mock("@/store/ui", () => ({ useUiStore: Object.assign(() => state.session, { getState: () => ({ daemonSession: state.session, bumpDaemonSession: () => { state.session++; }, addNotification: state.notify }) }) }));

function find(node: ReactNode, match: (props: Record<string, unknown>) => boolean): Record<string, unknown> | undefined {
  if (Array.isArray(node)) return node.map(child => find(child, match)).find(Boolean);
  if (!isValidElement<{children?: ReactNode}>(node)) return;
  if (match(node.props)) return node.props;
  return find(node.props.children, match);
}
function render() { state.cursor = 0; state.refCursor = 0; return BackupSettings(); }
function button(label: string) { return find(render(), props => props.children === label && !!props.onClick)!; }
function click(label: string) { (button(label).onClick as () => void)(); }
function input(id: string, value: string) { (find(render(), props => props.id === id)!.onChange as (event: {target: {value: string}}) => void)({target: {value}}); }

beforeEach(() => {
  state.values = []; state.refs = []; state.session = 1;
  state.invoke.mockReset(); state.pick.mockReset(); state.save.mockReset(); state.notify.mockReset();
  vi.stubGlobal("window", { dispatchEvent: vi.fn() });
});

it("requires matching backup passwords and cancels without daemon writes", async () => {
  click("backup.export");
  input("backup-passphrase", "first"); input("backup-second-passphrase", "different");
  expect(button("backup.export").disabled).toBe(true);
  input("backup-second-passphrase", "first");
  expect(button("backup.export").disabled).toBe(false);
  state.save.mockResolvedValue(null);
  click("backup.export");
  await vi.waitFor(() => expect(state.save).toHaveBeenCalledOnce());
  expect(state.invoke).not.toHaveBeenCalled();
});

it("shows the target, requires RESTORE and cancels a validated preview", async () => {
  click("backup.restore");
  input("backup-passphrase", "outer"); input("backup-second-passphrase", "inner");
  state.pick.mockResolvedValue("/tmp/example.kassiber");
  state.invoke.mockResolvedValue({kind: "ui.backup.preview", data: {token: "token", target_data_root: "/private/target/data", target: {books: ["Target book"], book_count: 1}, incoming: {books: ["Incoming book"], book_count: 1}, attachments_files: 2, replaces: ["attachments"]}});
  click("backup.preview");
  await vi.waitFor(() => expect(button("backup.replace")).toBeDefined());
  const text = JSON.stringify(render());
  expect(text).toContain("/private/target/data");
  expect(text).toContain("Target book");
  expect(text).toContain("Incoming book");
  expect(button("backup.replace").disabled).toBe(true);
  input("backup-confirmation", "RESTORE");
  expect(button("backup.replace").disabled).toBe(false);
  click("backup.cancel");
  expect(state.invoke).toHaveBeenLastCalledWith({kind: "ui.backup.cancel", args: {token: "token"}});
  expect(state.values).not.toContain("outer");
  expect(state.values).not.toContain("inner");
});

it("discards a late preview when the book switches during the request", async () => {
  click("backup.restore");
  input("backup-passphrase", "outer"); input("backup-second-passphrase", "inner");
  state.pick.mockResolvedValue("/tmp/example.kassiber");
  let finish: (value: unknown) => void = () => {};
  state.invoke.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValue({kind: "ui.backup.cancel", data: {cancelled: true}});
  click("backup.preview");
  await vi.waitFor(() => expect(state.invoke).toHaveBeenCalledOnce());
  state.session++;
  finish({kind: "ui.backup.preview", data: {token: "late"}});
  await vi.waitFor(() => expect(state.invoke).toHaveBeenLastCalledWith({kind: "ui.backup.cancel", args: {token: "late"}}));
  expect(button("backup.replace")).toBeUndefined();
});
