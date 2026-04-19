import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

ScrollView {
    id: root
    signal requestAddConnection()
    clip: true

    ColumnLayout {
        width: root.availableWidth
        spacing: theme.spacingLg

        Card {
            Layout.fillWidth: true
            fillColor: dashboardVM.hasData ? theme.cardAlt : theme.card

            ColumnLayout {
                anchors.fill: parent
                spacing: theme.spacingMd

                Text {
                    text: dashboardVM.hasData ? "Overview" : "Overview / Empty"
                    color: theme.accent
                    font.family: theme.bodyFont
                    font.pixelSize: 13
                    font.bold: true
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: dashboardVM.shellTitle
                    color: theme.ink
                    font.family: theme.displayFont
                    font.pixelSize: 32
                    font.bold: true
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: dashboardVM.shellBody
                    color: theme.inkMuted
                    font.family: theme.bodyFont
                    font.pixelSize: 14
                }

                RowLayout {
                    spacing: theme.spacingMd

                    PrimaryButton {
                        text: connectionsVM.ctaLabel
                        enabled: connectionsVM.canOpenAddConnection
                        onClicked: root.requestAddConnection()
                    }

                    Button {
                        text: "Tax Reports"
                        onClicked: dashboardVM.selectPage("reports")
                    }
                }
            }
        }

        GridLayout {
            Layout.fillWidth: true
            columns: width > 920 ? 4 : 2
            columnSpacing: theme.spacingMd
            rowSpacing: theme.spacingMd

            Repeater {
                model: dashboardVM.overviewMetrics

                Card {
                    Layout.fillWidth: true
                    padding: theme.spacingMd

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: theme.spacingSm

                        Text {
                            text: modelData["label"]
                            color: theme.inkMuted
                            font.family: theme.bodyFont
                            font.pixelSize: 12
                        }

                        Text {
                            text: modelData["value"]
                            color: theme.ink
                            font.family: theme.displayFont
                            font.pixelSize: 28
                            font.bold: true
                        }
                    }
                }
            }
        }

        GridLayout {
            Layout.fillWidth: true
            columns: width > 980 ? 3 : 1
            columnSpacing: theme.spacingMd
            rowSpacing: theme.spacingMd

            Repeater {
                model: dashboardVM.overviewHighlights

                Card {
                    Layout.fillWidth: true

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: theme.spacingSm

                        Text {
                            text: modelData["title"]
                            color: theme.inkMuted
                            font.family: theme.bodyFont
                            font.pixelSize: 12
                            font.bold: true
                        }

                        Text {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: modelData["value"]
                            color: theme.ink
                            font.family: theme.displayFont
                            font.pixelSize: 24
                            font.bold: true
                        }

                        Text {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: modelData["body"]
                            color: theme.inkMuted
                            font.family: theme.bodyFont
                            font.pixelSize: 13
                        }
                    }
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: theme.spacingMd

            Card {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: theme.spacingMd

                    Text {
                        text: "Connections Preview"
                        color: theme.ink
                        font.family: theme.displayFont
                        font.pixelSize: 22
                        font.bold: true
                    }

                    Repeater {
                        model: connectionsVM.items

                        Rectangle {
                            visible: index < 3
                            Layout.fillWidth: true
                            color: "transparent"
                            border.color: theme.cardBorder
                            border.width: 1
                            radius: theme.radiusMd
                            implicitHeight: 70

                            RowLayout {
                                anchors.fill: parent
                                anchors.margins: theme.spacingMd
                                spacing: theme.spacingMd

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 2

                                    Text {
                                        text: modelData["label"]
                                        color: theme.ink
                                        font.family: theme.displayFont
                                        font.pixelSize: 18
                                        font.bold: true
                                    }

                                    Text {
                                        text: modelData["subtitle"]
                                        color: theme.inkMuted
                                        font.family: theme.bodyFont
                                        font.pixelSize: 12
                                    }
                                }

                                Text {
                                    text: modelData["status_label"]
                                    color: modelData["status_tone"] === "ok" ? theme.ok : theme.warn
                                    font.family: theme.bodyFont
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                            }
                        }
                    }
                }
            }

            Card {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: theme.spacingMd

                    Text {
                        text: "Recent Transactions"
                        color: theme.ink
                        font.family: theme.displayFont
                        font.pixelSize: 22
                        font.bold: true
                    }

                    Repeater {
                        model: transactionsVM.items

                        RowLayout {
                            visible: index < 4
                            Layout.fillWidth: true
                            spacing: theme.spacingMd

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2

                                Text {
                                    text: modelData["title"]
                                    color: theme.ink
                                    font.family: theme.displayFont
                                    font.pixelSize: 16
                                    font.bold: true
                                }

                                Text {
                                    text: modelData["subtitle"]
                                    color: theme.inkMuted
                                    font.family: theme.bodyFont
                                    font.pixelSize: 12
                                }
                            }

                            Text {
                                text: modelData["amount_label"]
                                color: theme.ink
                                font.family: theme.bodyFont
                                font.pixelSize: 12
                            }
                        }
                    }
                }
            }
        }
    }
}
