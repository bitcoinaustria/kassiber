import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

// Static Tax / Capital-gains report view (mapped to dashboard "reports" route).
// No view-model bindings — inline mock data matching tax.jsx.
Item {
    id: root

    property bool hideSensitive: false

    signal requestBack()

    // Mock data --------------------------------------------------------------

    readonly property var mockYears: [2023, 2024, 2025, 2026]
    readonly property int activeYear: 2025
    readonly property string activeMethod: "fifo"

    readonly property var mockMethods: [
        { id: "fifo", label: "FIFO",        detail: "First-in, first-out \u00b7 Austrian default" },
        { id: "lifo", label: "LIFO",        detail: "Last-in, first-out" },
        { id: "hifo", label: "HIFO",        detail: "Highest-in, first-out (tax optimization)" },
        { id: "spec", label: "Specific ID", detail: "Per-lot selection" }
    ]

    readonly property var mockPolicies: [
        { label: "Treat internal transfers as non-taxable", checked: true },
        { label: "Apply 27.5 % KESt flat rate",             checked: true },
        { label: "Include Lightning channel fees as cost",  checked: true },
        { label: "Aggregate lots per UTXO set",             checked: false }
    ]

    readonly property var mockLots: [
        { acquired: "2022-03-18", disposed: "2025-11-04", holding: "> 1Y", longTerm: true,  sats: "12,000,000", cost: "3,851.20", proceeds: "8,204.18", gain: "+ 4,352.98" },
        { acquired: "2023-07-02", disposed: "2025-11-04", holding: "> 1Y", longTerm: true,  sats: "8,000,000",  cost: "2,412.00", proceeds: "5,469.45", gain: "+ 3,057.45" },
        { acquired: "2024-11-14", disposed: "2025-12-01", holding: "< 1Y", longTerm: false, sats: "3,500,000",  cost: "2,188.70", proceeds: "2,392.08", gain: "+ 203.38"   },
        { acquired: "2025-02-09", disposed: "2025-12-20", holding: "< 1Y", longTerm: false, sats: "1,800,000",  cost: "1,011.55", proceeds: "1,290.12", gain: "+ 278.57"   },
        { acquired: "2025-04-22", disposed: "2026-01-08", holding: "< 1Y", longTerm: false, sats: "900,000",    cost: "614.90",   proceeds: "635.14",   gain: "+ 20.24"    }
    ]

    readonly property var mockTotals: ({
        sats: "26,200,000",
        cost: "\u20ac 10,078.35",
        proceeds: "\u20ac 17,990.97",
        gain: "+ \u20ac 7,912.62",
        kest: "\u20ac 2,175.97"
    })

    property var activeYearValue: activeYear
    property string activeMethodValue: activeMethod
    property int activeStep: 1

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
                spacing: theme.spacingSm + 2

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Text {
                        text: "REPORT  \u00b7  \u00a727a EStG  \u00b7  AUSTRIA"
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                        font.letterSpacing: 1.4
                    }

                    Text {
                        text: "Capital gains"
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontDisplay - 4
                        font.weight: Font.DemiBold
                        font.letterSpacing: -0.4
                    }
                }

                Text {
                    text: "STEP " + root.activeStep + " / 2"
                    color: Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontCaption
                    font.letterSpacing: 0.8
                }

                ActionButton {
                    variant: "ghost"
                    size: "sm"
                    text: "\u2190 Back"
                    onClicked: root.requestBack()
                }
            }

            // ------------------------------------------------------------------
            // Body: 2-col
            // ------------------------------------------------------------------

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.pagePadding
                Layout.rightMargin: theme.pagePadding
                Layout.bottomMargin: theme.pagePadding
                columns: 2
                columnSpacing: theme.gridGap
                rowSpacing: theme.gridGap

                // ----- Left column: config -----
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 320
                    Layout.maximumWidth: 360
                    spacing: theme.gridGap

                    Card {
                        Layout.fillWidth: true
                        title: "Reporting period"

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: theme.gridGap

                            Flow {
                                Layout.fillWidth: true
                                spacing: theme.spacingXs

                                Repeater {
                                    model: root.mockYears

                                    Pill {
                                        text: String(modelData)
                                        active: root.activeYearValue === modelData
                                        tone: root.activeYearValue === modelData ? "ink" : "muted"
                                        onClicked: root.activeYearValue = modelData
                                    }
                                }
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: theme.spacingSm

                                DateInput {
                                    Layout.fillWidth: true
                                    label: "From"
                                    value: root.activeYearValue + "-01-01"
                                }

                                DateInput {
                                    Layout.fillWidth: true
                                    label: "To"
                                    value: root.activeYearValue + "-12-31"
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
                                model: root.mockMethods

                                RadioRow {
                                    Layout.fillWidth: true
                                    selected: root.activeMethodValue === modelData.id
                                    label: modelData.label
                                    description: modelData.detail
                                    onActivated: root.activeMethodValue = modelData.id
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
                            spacing: theme.spacingSm

                            Repeater {
                                model: root.mockPolicies

                                ToggleRow {
                                    Layout.fillWidth: true
                                    label: modelData.label
                                    checked: modelData.checked
                                }
                            }
                        }
                    }

                    ActionButton {
                        Layout.fillWidth: true
                        variant: "primary"
                        size: "lg"
                        text: "\u2192  Generate preview"
                        onClicked: root.activeStep = 2
                    }
                }

                // ----- Right column: preview -----
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 520
                    spacing: theme.gridGap

                    GridLayout {
                        Layout.fillWidth: true
                        columns: 4
                        columnSpacing: theme.gridGap
                        rowSpacing: theme.gridGap

                        StatTile {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.minimumWidth: 140
                            label: "Proceeds"
                            value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockTotals.proceeds
                            sub: root.mockLots.length + " disposals"
                        }

                        StatTile {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.minimumWidth: 140
                            label: "Cost basis"
                            value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockTotals.cost
                            sub: root.activeMethodValue.toUpperCase()
                        }

                        StatTile {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.minimumWidth: 140
                            label: "Net gain"
                            value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockTotals.gain
                            sub: root.activeYearValue + " tax year"
                            valueColor: theme.positive
                        }

                        StatTile {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.minimumWidth: 140
                            label: "KESt 27,5 %"
                            value: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockTotals.kest
                            sub: "Estimated liability"
                            valueColor: Design.accent(theme)
                        }
                    }

                    Card {
                        Layout.fillWidth: true
                        title: "Disposed lots \u00b7 " + root.activeYearValue
                        pad: false

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            spacing: 0

                            // Header
                            Rectangle {
                                Layout.fillWidth: true
                                Layout.preferredHeight: theme.rowHeightDefault - 4
                                color: "transparent"

                                Rectangle {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    height: 1
                                    color: Design.ink(theme)
                                }

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: theme.cardPadding
                                    anchors.rightMargin: theme.cardPadding
                                    spacing: theme.spacingSm + 2

                                    Text { Layout.preferredWidth: 86;  text: "ACQUIRED"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                    Text { Layout.preferredWidth: 86;  text: "DISPOSED"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                    Text { Layout.preferredWidth: 60;  text: "HOLDING";  color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
                                    Text { Layout.fillWidth: true;     text: "SATS";     color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                    Text { Layout.preferredWidth: 90;  text: "COST \u20ac";     color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                    Text { Layout.preferredWidth: 90;  text: "PROCEEDS \u20ac"; color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                    Text { Layout.preferredWidth: 80;  text: "GAIN \u20ac";     color: Design.ink3(theme); font.family: Design.sans(); font.pixelSize: theme.fontMicro; font.weight: Font.DemiBold; font.letterSpacing: 1.2; horizontalAlignment: Text.AlignRight }
                                }
                            }

                            // Body rows
                            Repeater {
                                model: root.mockLots

                                delegate: Rectangle {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: theme.rowHeightDefault - 4
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
                                        spacing: theme.spacingSm + 2

                                        Text { Layout.preferredWidth: 86; text: modelData.acquired; color: Design.ink2(theme); font.family: Design.mono(theme); font.pixelSize: theme.fontBodySmall }
                                        Text { Layout.preferredWidth: 86; text: modelData.disposed; color: Design.ink2(theme); font.family: Design.mono(theme); font.pixelSize: theme.fontBodySmall }

                                        Item {
                                            Layout.preferredWidth: 60
                                            Layout.preferredHeight: 18

                                            Rectangle {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: holdingText.implicitWidth + 12
                                                height: holdingText.implicitHeight + 4
                                                color: "transparent"
                                                border.color: modelData.longTerm ? theme.positive : Design.ink3(theme)
                                                border.width: 1

                                                Text {
                                                    id: holdingText
                                                    anchors.centerIn: parent
                                                    text: modelData.holding
                                                    color: modelData.longTerm ? theme.positive : Design.ink2(theme)
                                                    font.family: Design.mono(theme)
                                                    font.pixelSize: theme.fontMicro
                                                    font.letterSpacing: 1.0
                                                }
                                            }
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : modelData.sats
                                            color: Design.ink2(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontBodySmall
                                            horizontalAlignment: Text.AlignRight
                                        }

                                        Text {
                                            Layout.preferredWidth: 90
                                            text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : modelData.cost
                                            color: Design.ink2(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontBodySmall
                                            horizontalAlignment: Text.AlignRight
                                        }

                                        Text {
                                            Layout.preferredWidth: 90
                                            text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : modelData.proceeds
                                            color: Design.ink2(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontBodySmall
                                            horizontalAlignment: Text.AlignRight
                                        }

                                        Text {
                                            Layout.preferredWidth: 80
                                            text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : modelData.gain
                                            color: theme.positive
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontBodySmall
                                            horizontalAlignment: Text.AlignRight
                                        }
                                    }
                                }
                            }

                            // Totals row
                            Rectangle {
                                Layout.fillWidth: true
                                Layout.preferredHeight: theme.rowHeightDefault
                                color: Design.paper(theme)

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: theme.cardPadding
                                    anchors.rightMargin: theme.cardPadding
                                    spacing: theme.spacingSm + 2

                                    Text {
                                        Layout.preferredWidth: 232
                                        text: "Total"
                                        color: Design.ink(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        font.weight: Font.DemiBold
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockTotals.sats
                                        color: Design.ink(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        font.weight: Font.DemiBold
                                        horizontalAlignment: Text.AlignRight
                                    }

                                    Text {
                                        Layout.preferredWidth: 90
                                        text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockTotals.cost.replace("\u20ac ", "")
                                        color: Design.ink(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        font.weight: Font.DemiBold
                                        horizontalAlignment: Text.AlignRight
                                    }

                                    Text {
                                        Layout.preferredWidth: 90
                                        text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockTotals.proceeds.replace("\u20ac ", "")
                                        color: Design.ink(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        font.weight: Font.DemiBold
                                        horizontalAlignment: Text.AlignRight
                                    }

                                    Text {
                                        Layout.preferredWidth: 80
                                        text: root.hideSensitive ? "\u2022 \u2022 \u2022 \u2022" : root.mockTotals.gain
                                        color: theme.positive
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        font.weight: Font.DemiBold
                                        horizontalAlignment: Text.AlignRight
                                    }
                                }
                            }
                        }
                    }

                    GridLayout {
                        Layout.fillWidth: true
                        columns: 3
                        columnSpacing: theme.gridGap
                        rowSpacing: theme.gridGap

                        ExportFormat {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.minimumWidth: 160
                            formatName: "CSV"
                            subtitle: "Spreadsheet"
                            detail: "17 columns \u00b7 UTF-8"
                        }

                        ExportFormat {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.minimumWidth: 160
                            formatName: "PDF"
                            subtitle: "Human-readable"
                            detail: "4 pages \u00b7 \u00a727a format"
                            primary: true
                        }

                        ExportFormat {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.minimumWidth: 160
                            formatName: "JSON"
                            subtitle: "Envelope"
                            detail: "Machine-readable"
                        }
                    }
                }
            }
        }
    }
}
