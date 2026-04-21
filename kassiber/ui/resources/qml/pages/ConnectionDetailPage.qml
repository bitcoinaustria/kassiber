import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import Qt5Compat.GraphicalEffects

import "../components"
import "../components/Design.js" as Design

// Static Connection Detail view. Static mock data inline, no view-model
// bindings. Reuses Card / KV / ActionButton / ProtocolIcon / StatTile.
Item {
    id: root

    property bool hideSensitive: false

    signal requestAddConnection()
    signal requestBack()

    readonly property var selected: connectionsVM ? connectionsVM.selectedItem : ({})
    readonly property var selectedTxs: connectionsVM ? connectionsVM.selectedTransactions : []
    readonly property var selectedDetails: connectionsVM ? connectionsVM.selectedDetails : []

    readonly property int colDate: 80
    readonly property int colType: 90
    readonly property int colSats: 140
    readonly property int colEur: 120
    readonly property int colConf: 50

    function typeColor(t) {
        if (t === "Income") return theme.typeIncome
        if (t === "Expense") return theme.typeExpense
        if (t === "Transfer") return theme.typeTransfer
        if (t === "Swap") return theme.typeSwap
        if (t === "Consolidation") return theme.typeConsolidation
        if (t === "Rebalance") return theme.typeRebalance
        if (t === "Mint") return theme.typeMint
        if (t === "Melt") return theme.typeMelt
        if (t === "Fee") return theme.typeFee
        return Design.ink3(theme)
    }

    ScrollView {
        anchors.fill: parent
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: root.width
            spacing: theme.gridGap

            // ------------------------------------------------------------------
            // Header
            // ------------------------------------------------------------------

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.topMargin: theme.pagePadding
                spacing: theme.spacingSm + 6

                Button {
                    flat: true
                    padding: 0
                    implicitWidth: 32
                    implicitHeight: 32
                    hoverEnabled: true
                    onClicked: root.requestBack()

                    contentItem: Text {
                        anchors.fill: parent
                        text: "\u2039"
                        color: Design.ink(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontHeadingSm
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }

                    background: Rectangle {
                        color: "transparent"
                        border.color: Design.line(theme)
                        border.width: 1
                    }
                }

                ProtocolIcon {
                    kind: root.selected["kind"] || "xpub"
                    size: 40
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Text {
                        text: (root.selected["kind"] || "").toUpperCase() + "  \u00b7  CONNECTION"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                        font.letterSpacing: 1.4
                    }

                    Text {
                        text: root.selected["label"] || ""
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontDisplay - 2
                        font.weight: Font.DemiBold
                        font.letterSpacing: -0.4
                    }
                }

                ActionButton { variant: "secondary"; size: "sm"; text: "\u27f3 Sync" }
                ActionButton { variant: "secondary"; size: "sm"; text: "\u2193 Import labels" }
                ActionButton { variant: "secondary"; size: "sm"; text: "\u2191 Export labels" }

                Rectangle {
                    Layout.preferredWidth: 1
                    Layout.preferredHeight: 22
                    color: Design.line(theme)
                }

                ActionButton { variant: "ghost"; size: "sm"; text: "Edit" }
                ActionButton { variant: "danger"; size: "sm"; text: "Remove" }
            }

            // ------------------------------------------------------------------
            // 4-stat tile row
            // ------------------------------------------------------------------

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                columns: 4
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                StatTile {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 220
                    Layout.preferredWidth: 1
                    label: "Balance"
                    value: root.selected["balance_label"] || "\u2014"
                    sub: "Fiat not wired yet"
                    blurred: root.hideSensitive
                }

                StatTile {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 160
                    Layout.preferredWidth: 1
                    label: "Transactions"
                    value: root.selected["transaction_count_label"] || "0"
                    sub: "Included in snapshot"
                }

                StatTile {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 160
                    Layout.preferredWidth: 1
                    label: "Last activity"
                    value: root.selected["last_activity_label"] || "\u2014"
                    sub: "Most recent entry"
                    valueColor: root.selected["last_activity"] ? theme.positive : Design.ink3(theme)
                }

                StatTile {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 160
                    Layout.preferredWidth: 1
                    label: "Status"
                    value: root.selected["status_label"] || "\u2014"
                    sub: "Connection state"
                    valueColor: (root.selected["status_tone"] || "") === "ok" ? theme.positive : Design.accent(theme)
                }
            }

            // ------------------------------------------------------------------
            // 2-col row: Recent transactions | (Connection details + Derived addresses)
            // ------------------------------------------------------------------

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                columns: 2
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                // ----- Recent transactions -----
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 14
                    Layout.minimumWidth: 520
                    title: "Recent transactions"
                    pad: false

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        spacing: 0

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: theme.rowHeightDefault - 6
                            color: "transparent"

                            Rectangle {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                height: 1
                                color: Design.line(theme)
                            }

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: theme.cardPadding
                                anchors.rightMargin: theme.cardPadding
                                spacing: theme.spacingSm + 4

                                Text { Layout.preferredWidth: root.colDate; text: "DATE";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                Text { Layout.preferredWidth: root.colType; text: "TYPE";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                Item { Layout.fillWidth: true }
                                Text { Layout.preferredWidth: root.colSats; text: "SATS";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                Text { Layout.preferredWidth: root.colEur;  text: "\u20ac"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                Text { Layout.preferredWidth: root.colConf; text: "CONF";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                            }
                        }

                        Repeater {
                            model: root.selectedTxs

                            delegate: Rectangle {
                                Layout.fillWidth: true
                                Layout.preferredHeight: theme.rowHeightDefault
                                color: "transparent"

                                Rectangle {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    height: 1
                                    color: Design.line(theme)
                                }

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: theme.cardPadding
                                    anchors.rightMargin: theme.cardPadding
                                    spacing: theme.spacingSm + 4

                                    Text {
                                        Layout.preferredWidth: root.colDate
                                        text: modelData["occurred_on_label"] || ""
                                        color: Design.ink2(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                    }

                                    Text {
                                        Layout.preferredWidth: root.colType
                                        text: (modelData["type_label"] || modelData["kind_label"] || "").toUpperCase()
                                        color: root.typeColor(modelData["type_badge_tone"] || "")
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontMicro
                                        font.weight: Font.DemiBold
                                        font.letterSpacing: 1.0
                                    }

                                    Item { Layout.fillWidth: true }

                                    Text {
                                        Layout.preferredWidth: root.colSats
                                        text: modelData["amount_sats_signed_label"] || ""
                                        color: {
                                            var tone = modelData["type_tone"] || ""
                                            if (tone === "positive") return theme.positive
                                            if (tone === "negative") return Design.accent(theme)
                                            return Design.ink(theme)
                                        }
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                        layer.enabled: root.hideSensitive
                                        layer.effect: FastBlur { radius: 48 }
                                    }

                                    Text {
                                        Layout.preferredWidth: root.colEur
                                        text: modelData["fiat_label"] || ""
                                        color: Design.ink2(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                        layer.enabled: root.hideSensitive
                                        layer.effect: FastBlur { radius: 48 }
                                    }

                                    Text {
                                        Layout.preferredWidth: root.colConf
                                        text: "\u2014"
                                        color: Design.ink3(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        horizontalAlignment: Text.AlignRight
                                    }
                                }
                            }
                        }
                    }
                }

                // ----- Right column: Connection details + Derived addresses -----
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.horizontalStretchFactor: 10
                    Layout.minimumWidth: 320
                    spacing: theme.gridGap

                    Card {
                        Layout.fillWidth: true
                        title: "Connection details"

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.spacingSm + 4

                            Repeater {
                                model: root.selectedDetails

                                KV {
                                    Layout.fillWidth: true
                                    label: modelData["label"] || ""
                                    value: modelData["value"] || ""
                                    mono: (modelData["label"] || "") !== "Label"
                                              && (modelData["label"] || "") !== "Backend"
                                }
                            }
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: "Derived addresses"
                        subtitle: "Address derivation not wired yet."

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            implicitHeight: 140
                            color: "transparent"

                            Text {
                                anchors.centerIn: parent
                                text: "DATA UNAVAILABLE"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.4
                            }
                        }
                    }
                }
            }
        }
    }
}
