import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

Item {
    id: root

    property bool hideSensitive: false

    signal requestBack()

    readonly property string headerEyebrow: reportsVM ? reportsVM.headerEyebrow : "REPORT"
    readonly property string statusTitle: reportsVM ? reportsVM.statusTitle : ""
    readonly property string statusBody: reportsVM ? reportsVM.statusBody : ""
    readonly property var summaryCards: reportsVM ? reportsVM.summaryCards : []
    readonly property var methodOptions: reportsVM ? reportsVM.methodOptions : []
    readonly property var policyRows: reportsVM ? reportsVM.policyRows : []
    readonly property var previewRows: reportsVM ? reportsVM.previewRows : []
    readonly property var exportRows: reportsVM ? reportsVM.exportFormats : []
    readonly property string previewTitle: reportsVM ? reportsVM.previewTitle : "Preview unavailable"
    readonly property string previewSubtitle: reportsVM ? reportsVM.previewSubtitle : ""
    readonly property string previewEmptyHint: reportsVM ? reportsVM.previewEmptyHint : ""
    readonly property var yearOptions: {
        var seen = {}
        var output = []
        for (var i = 0; i < root.previewRows.length; i++) {
            var year = String(root.previewRows[i]["occurred_on_label"] || "").substring(0, 4)
            if (year.length === 4 && !seen[year]) {
                seen[year] = true
                output.push(year)
            }
        }
        if (output.length === 0) {
            output.push("2026")
        }
        return output
    }
    property string selectedYear: root.yearOptions.length > 0 ? root.yearOptions[0] : "2026"

    function toneColor(tone) {
        if (tone === "ok") return theme.positive
        if (tone === "warn") return Design.accent(theme)
        return Design.ink(theme)
    }

    ScrollView {
        anchors.fill: parent
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: root.width
            spacing: theme.gridGap

            SectionHeader {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.topMargin: theme.pagePadding
                label: root.headerEyebrow
                title: "Capital gains"
                subtitle: root.statusBody
                action: Component {
                    ActionButton {
                        variant: "ghost"
                        size: "sm"
                        text: "Back"
                        onClicked: root.requestBack()
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                spacing: theme.gridGap

                ColumnLayout {
                    Layout.preferredWidth: 300
                    Layout.alignment: Qt.AlignTop
                    spacing: theme.gridGap

                    Card {
                        Layout.fillWidth: true
                        title: "Reporting period"
                        subtitle: root.statusTitle

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.spacingSm + 2

                            Flow {
                                Layout.fillWidth: true
                                spacing: theme.spacingXs

                                Repeater {
                                    model: root.yearOptions

                                    Pill {
                                        text: modelData
                                        active: root.selectedYear === modelData
                                        interactive: false
                                        tone: root.selectedYear === modelData ? "ink" : "muted"
                                    }
                                }
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                Layout.topMargin: theme.spacingXs
                                spacing: theme.spacingSm

                                DateInput {
                                    Layout.fillWidth: true
                                    Layout.preferredWidth: 1
                                    interactive: false
                                    label: "FROM"
                                    value: root.selectedYear + "-01-01"
                                }

                                DateInput {
                                    Layout.fillWidth: true
                                    Layout.preferredWidth: 1
                                    interactive: false
                                    label: "TO"
                                    value: root.selectedYear + "-12-31"
                                }
                            }
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: "Cost-basis method"

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.spacingSm - 2

                            Repeater {
                                model: root.methodOptions

                                delegate: RadioRow {
                                    Layout.fillWidth: true
                                    interactive: false
                                    selected: !!modelData["selected"]
                                    label: modelData["label"] || ""
                                    description: modelData["detail"] || ""
                                }
                            }
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: "Policy"

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.spacingSm + 4

                            Repeater {
                                model: root.policyRows

                                delegate: KV {
                                    Layout.fillWidth: true
                                    label: modelData["label"] || ""
                                    value: modelData["detail"] || ""
                                    mono: false
                                }
                            }
                        }
                    }

                    ActionButton {
                        Layout.fillWidth: true
                        variant: "primary"
                        size: "lg"
                        text: "Generate preview"
                        enabled: false
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignTop
                    spacing: theme.gridGap

                    GridLayout {
                        Layout.fillWidth: true
                        columns: Math.max(1, Math.min(4, Math.floor(width / 200)))
                        columnSpacing: theme.gridGap
                        rowSpacing: theme.gridGap

                        Repeater {
                            model: root.summaryCards

                            delegate: StatTile {
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                Layout.preferredWidth: 1
                                label: modelData["label"] || ""
                                value: modelData["value"] || "-"
                                sub: modelData["detail"] || ""
                                valueColor: root.toneColor(modelData["tone"] || "")
                                blurred: root.hideSensitive
                            }
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: root.previewTitle
                        subtitle: root.previewSubtitle
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
                                visible: root.previewRows.length > 0

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

                                    Text { Layout.preferredWidth: 94; text: "DATE"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                    Text { Layout.preferredWidth: 132; text: "WALLET"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                    Text { Layout.preferredWidth: 88; text: "TYPE"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                    Text { Layout.preferredWidth: 124; text: "AMOUNT"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                    Text { Layout.preferredWidth: 112; text: "FIAT"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                    Text { Layout.fillWidth: true; text: "TAG"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                }
                            }

                            Repeater {
                                model: root.previewRows

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
                                            Layout.preferredWidth: 94
                                            text: modelData["occurred_on_label"] || ""
                                            color: Design.ink2(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontBodySmall
                                        }

                                        Text {
                                            Layout.preferredWidth: 132
                                            text: modelData["wallet"] || ""
                                            color: Design.ink(theme)
                                            font.family: Design.sans()
                                            font.pixelSize: theme.fontBody
                                            elide: Text.ElideRight
                                        }

                                        TypeBadge {
                                            Layout.preferredWidth: 88
                                            Layout.alignment: Qt.AlignLeft | Qt.AlignVCenter
                                            label: modelData["type_label"] || modelData["kind_label"] || ""
                                            tone: modelData["type_badge_tone"] || "muted"
                                        }

                                        Text {
                                            Layout.preferredWidth: 124
                                            text: modelData["amount_label"] || ""
                                            color: Design.ink(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontBodySmall
                                            horizontalAlignment: Text.AlignRight
                                            layer.enabled: root.hideSensitive
                                        }

                                        Text {
                                            Layout.preferredWidth: 112
                                            text: modelData["fiat_label"] || ""
                                            color: Design.ink2(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontBodySmall
                                            horizontalAlignment: Text.AlignRight
                                            elide: Text.ElideRight
                                            layer.enabled: root.hideSensitive
                                        }

                                        TagChip {
                                            Layout.fillWidth: true
                                            Layout.alignment: Qt.AlignVCenter
                                            label: modelData["tag_label"] || ""
                                        }
                                    }
                                }
                            }

                            Rectangle {
                                Layout.fillWidth: true
                                implicitHeight: 220
                                color: "transparent"
                                visible: root.previewRows.length === 0

                                Column {
                                    anchors.centerIn: parent
                                    spacing: 8

                                    Text {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        text: "DATA UNAVAILABLE"
                                        color: Design.ink3(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontCaption
                                        font.weight: Font.DemiBold
                                        font.letterSpacing: 1.4
                                    }

                                    Text {
                                        visible: root.previewEmptyHint.length > 0
                                        width: 320
                                        text: root.previewEmptyHint
                                        color: Design.ink3(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontBodySmall
                                        wrapMode: Text.WordWrap
                                        horizontalAlignment: Text.AlignHCenter
                                    }
                                }
                            }
                        }
                    }

                    GridLayout {
                        Layout.fillWidth: true
                        columns: width < 700 ? 1 : 3
                        columnSpacing: theme.gridGap
                        rowSpacing: theme.gridGap

                        Repeater {
                            model: root.exportRows

                            delegate: ExportFormat {
                                Layout.fillWidth: true
                                Layout.preferredWidth: 1
                                formatName: modelData["label"] || ""
                                subtitle: modelData["summary"] || ""
                                detail: modelData["detail"] || ""
                                primary: !!modelData["primary"]
                            }
                        }
                    }
                }
            }
        }
    }
}
