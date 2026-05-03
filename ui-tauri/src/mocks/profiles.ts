/**
 * Mock ledger + books fixture for the Books screen.
 *
 * Lifted from claude-design/components/strings.jsx (MOCK.workspaces) until
 * the Pydantic→JSON Schema pipeline (Phase 1.2 §2.2) generates fixtures
 * from real `kassiber.core.api.contracts` models. Shapes here become test
 * cases for the schema once it lands.
 */

export type WorkspaceKind = "Personal" | "Business" | "Household";

export type ProfileRole = "Owner" | "Treasurer" | "Auditor";

export interface Profile {
  id: string;
  name: string;
  role: ProfileRole;
  taxPolicy: string;
  accounts: number;
  wallets: number;
  lastOpened: string;
  active?: boolean;
}

export interface Workspace {
  id: string;
  name: string;
  kind: WorkspaceKind;
  currency: string;
  jurisdiction: string;
  created: string;
  profiles: Profile[];
}

export interface ProfilesSnapshot {
  workspaces: Workspace[];
  /** Currently active workspace id — valid even before a workspace has profiles. */
  activeWorkspaceId?: string;
  /** Currently active books/profile id — corresponds to the user's session. */
  activeProfileId: string;
}

export const MOCK_PROFILES: ProfilesSnapshot = {
  activeWorkspaceId: "w1",
  activeProfileId: "p1",
  workspaces: [
    {
      id: "w1",
      name: "My Books",
      kind: "Personal",
      currency: "EUR",
      jurisdiction: "Austria",
      created: "2024-03-12",
      profiles: [
        {
          id: "p1",
          name: "Alice",
          role: "Owner",
          taxPolicy: "Private · AT moving average",
          accounts: 4,
          wallets: 5,
          lastOpened: "Just now",
          active: true,
        },
        {
          id: "p2",
          name: "Alice · Self-employed",
          role: "Owner",
          taxPolicy: "Self-employed · FIFO · full income tax",
          accounts: 3,
          wallets: 2,
          lastOpened: "3 days ago",
        },
      ],
    },
    {
      id: "w2",
      name: "Hyperion GmbH",
      kind: "Business",
      currency: "EUR",
      jurisdiction: "Germany",
      created: "2024-09-01",
      profiles: [
        {
          id: "p3",
          name: "Hyperion GmbH · Operating",
          role: "Treasurer",
          taxPolicy: "Business · FIFO · corporate income tax",
          accounts: 6,
          wallets: 8,
          lastOpened: "Yesterday",
        },
        {
          id: "p4",
          name: "Hyperion GmbH · Treasury",
          role: "Treasurer",
          taxPolicy: "Business · FIFO",
          accounts: 2,
          wallets: 3,
          lastOpened: "1 week ago",
        },
      ],
    },
    {
      id: "w3",
      name: "Family",
      kind: "Household",
      currency: "CHF",
      jurisdiction: "Switzerland",
      created: "2025-02-18",
      profiles: [
        {
          id: "p5",
          name: "Household",
          role: "Owner",
          taxPolicy: "Private · shared",
          accounts: 2,
          wallets: 3,
          lastOpened: "2 weeks ago",
        },
      ],
    },
  ],
};
