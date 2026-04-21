import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

ScrollView {
    id: root
    clip: true

    contentWidth: availableWidth
    contentHeight: contentFrame.implicitHeight

    readonly property int pageWidth: Math.max(0, availableWidth)
    readonly property int panelWidth: Math.max(0, Math.min(Math.max(0, pageWidth - 48), 680))
    readonly property var privacyRows: settingsVM ? settingsVM.privacyRows : []
    readonly property var lockRows: settingsVM ? settingsVM.lockRows : []
    readonly property var timeoutOptions: settingsVM ? settingsVM.idleOptions : []
    readonly property var dataActions: settingsVM ? settingsVM.dataActions : []
    readonly property var syncRows: {
        var source = settingsVM ? (settingsVM.backendRows || []) : []
        var rows = []
        for (var i = 0; i < source.length; ++i) {
            rows.push({
                "name": source[i]["label"],
                "value": source[i]["value"],
                "state": source[i]["status"]
            })
        }
        return rows
    }
    readonly property var pathCards: {
        var source = settingsVM ? (settingsVM.cards || []) : []
        var rows = []
        for (var i = 0; i < source.length; ++i) {
            if (String(source[i]["label"] || "") === "active profile") {
                continue
            }
            rows.push(source[i])
        }
        return rows
    }

    function cardByLabel(label) {
        var source = settingsVM ? (settingsVM.cards || []) : []
        for (var i = 0; i < source.length; ++i) {
            if (String(source[i]["label"] || "") === label) {
                return source[i]
            }
        }
        return null
    }

    function cardValue(label, fallback) {
        var card = cardByLabel(label)
        if (!card || !String(card["value"] || "").length) {
            return fallback || ""
        }
        return card["value"]
    }

    function stateColor(state) {
        return (state === "managed" || state === "local" || state === "active") ? theme.ok : theme.warn
    }

    Item {
        id: contentFrame
        width: root.availableWidth
        implicitHeight: contentColumn.implicitHeight

        Column {
            id: contentColumn
            width: root.panelWidth
            x: (parent.width - width) / 2
            spacing: theme.spacingLg

            Card {
                width: parent.width
                fillColor: theme.cardAlt
                padding: 20

                ColumnLayout {
                    width: parent.width
                    spacing: 18

                    Text {
                        Layout.fillWidth: true
                        text: "Settings"
                        color: theme.ink
                        font.family: theme.displayFont
                        font.pixelSize: 32
                        font.weight: Font.Normal
                    }

                    Text {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        text: "Privacy, lock, data, and sync settings are shown here in the same sectional order as the Claude export. Actions stay read-only until the desktop shell wires them through."
                        color: theme.ink2
                        font.family: theme.sansFont
                        font.pixelSize: 13
                        lineHeight: 1.5
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        visible: root.cardByLabel("active profile") !== null
                        color: theme.paper2
                        border.color: theme.line
                        border.width: 1
                        radius: theme.radiusSm
                        implicitHeight: profileColumn.implicitHeight + 18

                        ColumnLayout {
                            id: profileColumn
                            width: parent.width - 20
                            x: 10
                            y: 10
                            spacing: 4

                            Text {
                                text: "ACTIVE PROFILE"
                                color: theme.ink3
                                font.family: theme.monoFont
                                font.pixelSize: 10
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.2
                            }

                            Text {
                                Layout.fillWidth: true
                                wrapMode: Text.WordWrap
                                text: root.cardValue("active profile", "")
                                color: theme.ink
                                font.family: theme.monoFont
                                font.pixelSize: 11
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        Text {
                            text: "Privacy"
                            color: theme.ink3
                            font.family: theme.monoFont
                            font.pixelSize: 10
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }

                        Rule {
                            Layout.fillWidth: true
                            ruleColor: theme.line
                        }

                        Repeater {
                            model: root.privacyRows

                            Item {
                                Layout.fillWidth: true
                                implicitHeight: privacyRow.implicitHeight

                                RowLayout {
                                    id: privacyRow
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    spacing: theme.spacingMd

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 3

                                        Text {
                                            Layout.fillWidth: true
                                            text: modelData["label"]
                                            color: theme.ink
                                            font.family: theme.sansFont
                                            font.pixelSize: 13
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            wrapMode: Text.WordWrap
                                            text: modelData["detail"] || modelData["sub"] || ""
                                            color: theme.ink3
                                            font.family: theme.sansFont
                                            font.pixelSize: 11
                                            lineHeight: 1.45
                                        }
                                    }

                                    Rectangle {
                                        Layout.alignment: Qt.AlignVCenter
                                        width: 36
                                        height: 20
                                        color: modelData["enabled"] ? theme.ink : theme.line2
                                        radius: 10

                                        Rectangle {
                                            x: modelData["enabled"] ? 18 : 2
                                            y: 2
                                            width: 16
                                            height: 16
                                            radius: 8
                                            color: theme.paper2
                                        }
                                    }
                                }
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        Text {
                            text: "App lock"
                            color: theme.ink3
                            font.family: theme.monoFont
                            font.pixelSize: 10
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }

                        Rule {
                            Layout.fillWidth: true
                            ruleColor: theme.line
                        }

                        Repeater {
                            model: root.lockRows

                            Item {
                                Layout.fillWidth: true
                                implicitHeight: lockRow.implicitHeight

                                RowLayout {
                                    id: lockRow
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    spacing: theme.spacingMd

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 3

                                        Text {
                                            Layout.fillWidth: true
                                            text: modelData["label"]
                                            color: theme.ink
                                            font.family: theme.sansFont
                                            font.pixelSize: 13
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            wrapMode: Text.WordWrap
                                            text: modelData["detail"] || modelData["sub"] || ""
                                            color: theme.ink3
                                            font.family: theme.sansFont
                                            font.pixelSize: 11
                                            lineHeight: 1.45
                                        }
                                    }

                                    Rectangle {
                                        Layout.alignment: Qt.AlignVCenter
                                        width: 36
                                        height: 20
                                        color: modelData["enabled"] ? theme.ink : theme.line2
                                        radius: 10

                                        Rectangle {
                                            x: modelData["enabled"] ? 18 : 2
                                            y: 2
                                            width: 16
                                            height: 16
                                            radius: 8
                                            color: theme.paper2
                                        }
                                    }
                                }
                            }
                        }

                        Item {
                            Layout.fillWidth: true
                            implicitHeight: timeoutBlock.implicitHeight

                            ColumnLayout {
                                id: timeoutBlock
                                anchors.left: parent.left
                                anchors.right: parent.right
                                spacing: 8

                                Text {
                                    text: "Idle timeout"
                                    color: theme.ink2
                                    font.family: theme.sansFont
                                    font.pixelSize: 12
                                }

                                Flow {
                                    width: parent.width
                                    spacing: theme.spacingSm

                                    Repeater {
                                        model: root.timeoutOptions

                                        Pill {
                                            text: modelData + "m"
                                            active: settingsVM ? modelData === settingsVM.activeIdleOption : false
                                            tone: settingsVM && modelData === settingsVM.activeIdleOption ? "ink" : "muted"
                                        }
                                    }
                                }
                            }
                        }

                        Flow {
                            Layout.fillWidth: true
                            spacing: theme.spacingSm

                            SecondaryButton {
                                size: "sm"
                                text: "Lock now"
                            }

                            GhostButton {
                                size: "sm"
                                text: "Change passphrase"
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        Text {
                            text: "Data"
                            color: theme.ink3
                            font.family: theme.monoFont
                            font.pixelSize: 10
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }

                        Rule {
                            Layout.fillWidth: true
                            ruleColor: theme.line
                        }

                        GridLayout {
                            Layout.fillWidth: true
                            columns: root.panelWidth >= 620 ? 3 : 1
                            columnSpacing: theme.spacingSm
                            rowSpacing: theme.spacingSm

                            Repeater {
                                model: root.dataActions

                                SecondaryButton {
                                    Layout.fillWidth: true
                                    size: "md"
                                    text: modelData["label"]
                                }
                            }
                        }

                        Repeater {
                            model: root.pathCards

                            Rectangle {
                                Layout.fillWidth: true
                                color: theme.paper2
                                border.color: theme.line
                                border.width: 1
                                radius: theme.radiusSm
                                implicitHeight: pathColumn.implicitHeight + 16

                                ColumnLayout {
                                    id: pathColumn
                                    width: parent.width - 16
                                    x: 8
                                    y: 8
                                    spacing: 3

                                    Text {
                                        text: modelData["label"]
                                        color: theme.ink3
                                        font.family: theme.monoFont
                                        font.pixelSize: 10
                                        font.weight: Font.DemiBold
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        wrapMode: Text.WrapAnywhere
                                        text: modelData["value"]
                                        color: theme.ink
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        wrapMode: Text.WordWrap
                                        text: modelData["hint"] || ""
                                        color: theme.ink3
                                        font.family: theme.sansFont
                                        font.pixelSize: 11
                                    }
                                }
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        Text {
                            text: "Sync backends"
                            color: theme.ink3
                            font.family: theme.monoFont
                            font.pixelSize: 10
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }

                        Rule {
                            Layout.fillWidth: true
                            ruleColor: theme.line
                        }

                        Repeater {
                            model: root.syncRows

                            Rectangle {
                                Layout.fillWidth: true
                                color: "transparent"
                                border.color: theme.line
                                border.width: 1
                                implicitHeight: 42

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 10
                                    anchors.rightMargin: 10
                                    spacing: theme.spacingSm

                                    Rectangle {
                                        width: 6
                                        height: 6
                                        radius: 3
                                        color: root.stateColor(modelData["state"])
                                    }

                                    Text {
                                        Layout.preferredWidth: 150
                                        text: modelData["name"]
                                        color: theme.ink
                                        font.family: theme.monoFont
                                        font.pixelSize: 11
                                        elide: Text.ElideRight
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData["value"]
                                        color: theme.ink3
                                        font.family: theme.monoFont
                                        font.pixelSize: 10
                                        elide: Text.ElideRight
                                    }

                                    Text {
                                        text: String(modelData["state"] || "").toUpperCase()
                                        color: root.stateColor(modelData["state"])
                                        font.family: theme.monoFont
                                        font.pixelSize: 9
                                        font.weight: Font.DemiBold
                                        font.letterSpacing: 1.1
                                    }
                                }
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        Text {
                            text: "Danger zone"
                            color: theme.ink3
                            font.family: theme.monoFont
                            font.pixelSize: 10
                            font.weight: Font.DemiBold
                            font.letterSpacing: 1.4
                        }

                        Rule {
                            Layout.fillWidth: true
                            ruleColor: theme.line
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: theme.spacingMd

                            DangerButton {
                                size: "sm"
                                text: "Reset workspace"
                            }

                            Text {
                                Layout.fillWidth: true
                                wrapMode: Text.WordWrap
                                text: "Kept visible for parity with the export. Destructive actions remain intentionally disabled in the current desktop shell."
                                color: theme.ink3
                                font.family: theme.sansFont
                                font.pixelSize: 11
                                lineHeight: 1.45
                            }
                        }
                    }
                }
            }
        }
    }
}
