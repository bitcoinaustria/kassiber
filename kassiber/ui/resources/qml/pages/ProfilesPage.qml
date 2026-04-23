import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

// Static Profiles screen. Inline mock data matching profiles.jsx.
Item {
    id: root

    property string activeProfileId: "p1"

    signal requestBack()
    signal requestNewProfile(string workspaceId)
    signal requestNewWorkspace()

    readonly property var mockWorkspaces: [
        {
            id: "w1", name: "My Books", kind: "Personal",
            currency: "EUR", jurisdiction: "Austria", created: "2024-03-12",
            profiles: [
                { id: "p1", name: "Alice",                role: "Owner", taxPolicy: "Private \u00b7 \u00a727b EStG \u00b7 1 year spec.", buckets: 4, wallets: 5, lastOpened: "Just now" },
                { id: "p2", name: "Alice \u00b7 Self-employed", role: "Owner", taxPolicy: "Self-employed \u00b7 FIFO \u00b7 full income tax", buckets: 3, wallets: 2, lastOpened: "3 days ago" }
            ]
        },
        {
            id: "w2", name: "Hyperion OG", kind: "Business",
            currency: "EUR", jurisdiction: "Austria", created: "2024-09-01",
            profiles: [
                { id: "p3", name: "Hyperion OG \u00b7 Operating", role: "Treasurer", taxPolicy: "Business \u00b7 FIFO \u00b7 K\u00f6St + KESt split",   buckets: 6, wallets: 8, lastOpened: "Yesterday" },
                { id: "p4", name: "Hyperion OG \u00b7 Treasury",  role: "Treasurer", taxPolicy: "Business \u00b7 FIFO \u00b7 long-term hold",           buckets: 2, wallets: 3, lastOpened: "1 week ago" }
            ]
        },
        {
            id: "w3", name: "Family", kind: "Household",
            currency: "EUR", jurisdiction: "Austria", created: "2025-02-18",
            profiles: [
                { id: "p5", name: "Household", role: "Owner", taxPolicy: "Private \u00b7 shared \u00b7 1 year spec.", buckets: 2, wallets: 3, lastOpened: "2 weeks ago" }
            ]
        }
    ]

    readonly property int workspaceCount: mockWorkspaces.length
    readonly property int profileCount: {
        var total = 0
        for (var i = 0; i < mockWorkspaces.length; i++) total += mockWorkspaces[i].profiles.length
        return total
    }

    ScrollView {
        anchors.fill: parent
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: root.width
            spacing: theme.gridGap + 6

            // ------------------------------------------------------------------
            // Header
            // ------------------------------------------------------------------

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding + 4
                Layout.rightMargin: theme.pagePadding + 4
                Layout.topMargin: theme.pagePadding + 4
                spacing: theme.spacingSm + 2

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4

                    Text {
                        text: "IDENTITY  \u00b7  " + root.workspaceCount + " WORKSPACES  \u00b7  " + root.profileCount + " PROFILES"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                        font.letterSpacing: 1.4
                    }

                    Text {
                        text: "Switch profile"
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontDisplay - 4
                        font.weight: Font.DemiBold
                        font.letterSpacing: -0.4
                    }

                    Text {
                        Layout.fillWidth: true
                        Layout.maximumWidth: 640
                        text: "Each profile keeps its own books, tax policy, wallet buckets and wallets. " +
                              "Nothing is shared across profiles \u2014 switching reloads the ledger in read-only mode."
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBody + 1
                        wrapMode: Text.WordWrap
                    }
                }

                ActionButton { variant: "ghost";     size: "sm"; text: "\u2190 Back";       onClicked: root.requestBack() }
                ActionButton { variant: "secondary"; size: "sm"; text: "+ Profile";         onClicked: root.requestNewProfile("") }
                ActionButton { variant: "primary";   size: "sm"; text: "+ Workspace";       onClicked: root.requestNewWorkspace() }
            }

            // ------------------------------------------------------------------
            // Workspaces
            // ------------------------------------------------------------------

            Repeater {
                model: root.mockWorkspaces

                delegate: ColumnLayout {
                    Layout.fillWidth: true
                    Layout.leftMargin: theme.pagePadding + 4
                    Layout.rightMargin: theme.pagePadding + 4
                    spacing: theme.spacingSm + 2

                    // Workspace header
                    Item {
                        Layout.fillWidth: true
                        Layout.preferredHeight: wsHeader.implicitHeight + 12

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            height: 1
                            color: Design.ink(theme)
                        }

                        RowLayout {
                            id: wsHeader
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            anchors.topMargin: 4
                            spacing: theme.spacingSm + 4

                            Rectangle {
                                Layout.preferredWidth: 12
                                Layout.preferredHeight: 12
                                color: "transparent"
                                border.color: Design.ink(theme)
                                border.width: 1
                            }

                            Text {
                                text: modelData.name
                                color: Design.ink(theme)
                                font.family: Design.sans()
                                font.pixelSize: theme.fontHeadingSm + 3
                                font.letterSpacing: -0.1
                            }

                            Text {
                                Layout.alignment: Qt.AlignBaseline
                                text: (modelData.kind + " \u00b7 " + modelData.currency + " \u00b7 " + modelData.jurisdiction + " \u00b7 SINCE " + modelData.created).toUpperCase()
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontMicro
                                font.letterSpacing: 1.0
                            }

                            Item { Layout.fillWidth: true }

                            Text {
                                text: modelData.profiles.length + (modelData.profiles.length === 1 ? " profile" : " profiles")
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontMicro
                                font.letterSpacing: 1.0
                            }
                        }
                    }

                    // Profile grid (2-col)
                    GridLayout {
                        Layout.fillWidth: true
                        columns: 2
                        columnSpacing: theme.gridGap
                        rowSpacing: theme.gridGap

                        Repeater {
                            model: modelData.profiles

                            ProfileCard {
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                Layout.minimumWidth: 320
                                profileName: modelData.name
                                role: modelData.role
                                lastOpened: modelData.lastOpened
                                taxPolicy: modelData.taxPolicy
                                bucketsCount: modelData.buckets
                                walletsCount: modelData.wallets
                                active: root.activeProfileId === modelData.id
                                onClicked: root.activeProfileId = modelData.id
                            }
                        }

                        // Dashed "new profile" tile
                        Button {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.minimumWidth: 320
                            flat: true
                            padding: 0
                            hoverEnabled: true
                            implicitHeight: 150
                            onClicked: root.requestNewProfile(modelData.id)

                            contentItem: Column {
                                anchors.centerIn: parent
                                spacing: 4

                                Text {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: "+"
                                    color: Design.ink2(theme)
                                    font.family: Design.sans()
                                    font.pixelSize: theme.fontHeadingLg
                                }

                                Text {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: "New profile in " + modelData.name
                                    color: Design.ink2(theme)
                                    font.family: Design.sans()
                                    font.pixelSize: theme.fontBody
                                }

                                Text {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: "INHERIT TAX DEFAULTS"
                                    color: Design.ink3(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontMicro
                                    font.letterSpacing: 1.2
                                }
                            }

                            background: Rectangle {
                                color: "transparent"
                                border.color: Design.line2(theme)
                                border.width: 1
                            }
                        }
                    }
                }
            }

            // ------------------------------------------------------------------
            // New workspace footer
            // ------------------------------------------------------------------

            Button {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding + 4
                Layout.rightMargin: theme.pagePadding + 4
                Layout.bottomMargin: theme.pagePadding + 4
                flat: true
                padding: 0
                hoverEnabled: true
                implicitHeight: 62
                onClicked: root.requestNewWorkspace()

                contentItem: RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: theme.cardPadding + 4
                    anchors.rightMargin: theme.cardPadding + 4
                    spacing: theme.spacingSm + 4

                    Text {
                        text: "+"
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontHeadingLg + 4
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2

                        Text {
                            text: "New workspace"
                            color: Design.ink(theme)
                            font.family: Design.sans()
                            font.pixelSize: theme.fontHeadingSm
                        }

                        Text {
                            text: "Separate books \u00b7 separate tax policy \u00b7 separate backups"
                            color: Design.ink3(theme)
                            font.family: Design.sans()
                            font.pixelSize: theme.fontBody
                        }
                    }

                    Item { Layout.fillWidth: true }
                }

                background: Rectangle {
                    color: "transparent"
                    border.color: Design.line2(theme)
                    border.width: 1
                }
            }
        }
    }
}
