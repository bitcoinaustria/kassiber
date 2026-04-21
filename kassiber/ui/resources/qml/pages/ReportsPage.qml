import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

Item {
    id: root

    property bool hideSensitive: false

    signal requestBack()

    readonly property var methodOptions: reportsVM ? reportsVM.methodOptions : []
    readonly property var exportRows: reportsVM ? reportsVM.exportFormats : []
    readonly property var yearOptions: ["2023", "2024", "2025", "2026"]
    property string selectedYear: "2025"

    readonly property var policyToggles: [
        { label: "Treat internal transfers as non-taxable", enabled: true },
        { label: "Apply 27.5% KESt flat rate", enabled: true },
        { label: "Include Lightning channel fees as cost", enabled: true },
        { label: "Aggregate lots per UTXO set", enabled: false }
    ]

    readonly property var summaryTiles: [
        { label: "Proceeds", tone: "neutral", detail: "5 disposals" },
        { label: "Cost basis", tone: "neutral", detail: "FIFO" },
        { label: "Net gain", tone: "ok", detail: root.selectedYear + " tax year" },
        { label: "KESt 27.5%", tone: "warn", detail: "Estimated liability" }
    ]

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

            // ------------------------------------------------------------------
            // Header
            // ------------------------------------------------------------------

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.topMargin: theme.pagePadding

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6

                    Text {
                        Layout.fillWidth: true
                        text: "REPORT  \u00b7  \u00a727A ESTG  \u00b7  AUSTRIA"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                        font.letterSpacing: 1.4
                    }

                    Text {
                        Layout.fillWidth: true
                        text: "Capital gains"
                        color: Design.ink(theme)
                        font.family: Design.serif(theme)
                        font.pixelSize: 28
                        font.weight: Font.Normal
                        font.letterSpacing: -0.2
                        wrapMode: Text.WordWrap
                    }
                }

                ColumnLayout {
                    Layout.alignment: Qt.AlignTop | Qt.AlignRight
                    spacing: 6

                    Text {
                        Layout.alignment: Qt.AlignRight
                        text: "STEP 1 / 2"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: 10
                        font.weight: Font.DemiBold
                        font.letterSpacing: 1.4
                    }

                    ActionButton {
                        Layout.alignment: Qt.AlignRight
                        variant: "ghost"
                        size: "sm"
                        text: "\u2190 Back"
                        onClicked: root.requestBack()
                    }
                }
            }

            // ------------------------------------------------------------------
            // Main: 2-column split (left controls / right preview)
            // ------------------------------------------------------------------

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                spacing: theme.gridGap

                // ---------- LEFT COLUMN: controls ----------
                ColumnLayout {
                    Layout.preferredWidth: 300
                    Layout.alignment: Qt.AlignTop
                    spacing: theme.gridGap

                    // Reporting period
                    Card {
                        Layout.fillWidth: true
                        title: "Reporting period"

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
                                        tone: root.selectedYear === modelData ? "ink" : "muted"
                                        onClicked: root.selectedYear = modelData
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
                                    label: "FROM"
                                    value: root.selectedYear + "-01-01"
                                }

                                DateInput {
                                    Layout.fillWidth: true
                                    Layout.preferredWidth: 1
                                    label: "TO"
                                    value: root.selectedYear + "-12-31"
                                }
                            }
                        }
                    }

                    // Cost-basis method
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
                                    selected: !!modelData["selected"]
                                    label: modelData["label"] || ""
                                    description: modelData["detail"] || ""
                                }
                            }
                        }
                    }

                    // Policy
                    Card {
                        Layout.fillWidth: true
                        title: "Policy"

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.spacingSm + 2

                            Repeater {
                                model: root.policyToggles

                                delegate: ToggleSwitch {
                                    Layout.fillWidth: true
                                    text: modelData["label"] || ""
                                    checked: !!modelData["enabled"]
                                }
                            }
                        }
                    }

                    // Generate preview
                    ActionButton {
                        Layout.fillWidth: true
                        variant: "primary"
                        size: "lg"
                        text: "\u2192  Generate preview"
                        enabled: false
                    }
                }

                // ---------- RIGHT COLUMN: preview ----------
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignTop
                    spacing: theme.gridGap

                    // Summary tiles
                    GridLayout {
                        Layout.fillWidth: true
                        columns: Math.max(1, Math.min(4, Math.floor(width / 200)))
                        columnSpacing: theme.gridGap
                        rowSpacing: theme.gridGap

                        Repeater {
                            model: root.summaryTiles

                            delegate: StatTile {
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                Layout.preferredWidth: 1
                                label: modelData["label"] || ""
                                value: "\u2014"
                                sub: modelData["detail"] || ""
                                valueColor: Design.ink3(theme)
                                blurred: root.hideSensitive
                            }
                        }
                    }

                    // Disposed lots table (placeholder)
                    Card {
                        Layout.fillWidth: true
                        title: "Disposed lots \u00b7 " + root.selectedYear
                        subtitle: "AT tax processing not yet available."

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            implicitHeight: 260
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

                    // Export tiles
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
