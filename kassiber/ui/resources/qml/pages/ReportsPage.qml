import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

ScrollView {
    id: root
    clip: true

    ColumnLayout {
        width: root.availableWidth
        spacing: theme.spacingLg

        Card {
            Layout.fillWidth: true
            fillColor: theme.cardAlt

            ColumnLayout {
                anchors.fill: parent
                spacing: theme.spacingMd

                Text {
                    text: "Tax Reports View"
                    color: theme.accent
                    font.family: theme.bodyFont
                    font.pixelSize: 13
                    font.bold: true
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: reportsVM.statusTitle
                    color: theme.ink
                    font.family: theme.displayFont
                    font.pixelSize: 32
                    font.bold: true
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: reportsVM.statusBody
                    color: theme.inkMuted
                    font.family: theme.bodyFont
                    font.pixelSize: 14
                }
            }
        }

        GridLayout {
            Layout.fillWidth: true
            columns: width > 920 ? 3 : 1
            columnSpacing: theme.spacingMd
            rowSpacing: theme.spacingMd

            Repeater {
                model: reportsVM.summaryCards

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
                            Layout.fillWidth: true
                            wrapMode: Text.WrapAnywhere
                            text: modelData["value"]
                            color: theme.ink
                            font.family: theme.displayFont
                            font.pixelSize: 24
                            font.bold: true
                        }
                    }
                }
            }
        }

        Repeater {
            model: reportsVM.items

            Card {
                Layout.fillWidth: true

                RowLayout {
                    anchors.fill: parent
                    spacing: theme.spacingMd

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: theme.spacingSm

                        Text {
                            text: modelData["label"]
                            color: theme.ink
                            font.family: theme.displayFont
                            font.pixelSize: 24
                            font.bold: true
                        }

                        Text {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: modelData["summary"]
                            color: theme.inkMuted
                            font.family: theme.bodyFont
                            font.pixelSize: 13
                        }
                    }

                    Rectangle {
                        width: 118
                        height: 32
                        radius: 16
                        color: "transparent"
                        border.color: modelData["status_tone"] === "ok" ? theme.ok : theme.warn
                        border.width: 1

                        Text {
                            anchors.centerIn: parent
                            text: modelData["status"]
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
}
