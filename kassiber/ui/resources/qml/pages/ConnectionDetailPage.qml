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

        RowLayout {
            Layout.fillWidth: true
            spacing: theme.spacingMd

            Text {
                text: "Connection Detail"
                color: theme.ink
                font.family: theme.displayFont
                font.pixelSize: 30
                font.bold: true
            }

            Item {
                Layout.fillWidth: true
            }

            PrimaryButton {
                text: connectionsVM.ctaLabel
                enabled: connectionsVM.canOpenAddConnection
                onClicked: root.requestAddConnection()
            }
        }

        Card {
            Layout.fillWidth: true
            visible: connectionsVM.isEmpty

            ColumnLayout {
                anchors.fill: parent
                spacing: theme.spacingMd

                Text {
                    text: "No connections yet"
                    color: theme.ink
                    font.family: theme.displayFont
                    font.pixelSize: 26
                    font.bold: true
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "This screen is ready for the Connection Detail mockup, but it still needs at least one wallet or import to render live data."
                    color: theme.inkMuted
                    font.family: theme.bodyFont
                    font.pixelSize: 14
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: !connectionsVM.isEmpty
            spacing: theme.spacingMd

            Card {
                Layout.preferredWidth: 330
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: theme.spacingSm

                    Text {
                        text: "Connections"
                        color: theme.ink
                        font.family: theme.displayFont
                        font.pixelSize: 22
                        font.bold: true
                    }

                    Repeater {
                        model: connectionsVM.items

                        Button {
                            Layout.fillWidth: true
                            text: modelData["label"]
                            onClicked: connectionsVM.selectConnection(modelData["id"])

                            contentItem: Column {
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

                            background: Rectangle {
                                color: connectionsVM.selectedId === modelData["id"] ? theme.cardAlt : "transparent"
                                radius: theme.radiusMd
                                border.color: theme.cardBorder
                                border.width: 1
                            }

                            implicitHeight: 72
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

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: theme.spacingMd

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2

                            Text {
                                text: connectionsVM.selectedItem["label"] || ""
                                color: theme.ink
                                font.family: theme.displayFont
                                font.pixelSize: 30
                                font.bold: true
                            }

                            Text {
                                text: connectionsVM.selectedItem["subtitle"] || ""
                                color: theme.inkMuted
                                font.family: theme.bodyFont
                                font.pixelSize: 13
                            }
                        }

                        Rectangle {
                            width: 110
                            height: 32
                            radius: 16
                            color: "transparent"
                            border.color: connectionsVM.selectedItem["status_tone"] === "ok" ? theme.ok : theme.warn
                            border.width: 1

                            Text {
                                anchors.centerIn: parent
                                text: connectionsVM.selectedItem["status_label"] || ""
                                color: connectionsVM.selectedItem["status_tone"] === "ok" ? theme.ok : theme.warn
                                font.family: theme.bodyFont
                                font.pixelSize: 12
                                font.bold: true
                            }
                        }
                    }

                    GridLayout {
                        Layout.fillWidth: true
                        columns: width > 760 ? 2 : 1
                        columnSpacing: theme.spacingMd
                        rowSpacing: theme.spacingMd

                        Repeater {
                            model: connectionsVM.selectedDetails

                            Card {
                                Layout.fillWidth: true
                                padding: theme.spacingMd
                                fillColor: theme.cardAlt

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
                                        font.family: theme.bodyFont
                                        font.pixelSize: 13
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
