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

        Text {
            text: "Transaction View"
            color: theme.ink
            font.family: theme.displayFont
            font.pixelSize: 30
            font.bold: true
        }

        Card {
            Layout.fillWidth: true
            visible: transactionsVM.isEmpty

            ColumnLayout {
                anchors.fill: parent
                spacing: theme.spacingMd

                Text {
                    text: "No transactions yet"
                    color: theme.ink
                    font.family: theme.displayFont
                    font.pixelSize: 26
                    font.bold: true
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Once a wallet syncs or an import lands, this page becomes the home for the Transaction View mockup."
                    color: theme.inkMuted
                    font.family: theme.bodyFont
                    font.pixelSize: 14
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: !transactionsVM.isEmpty
            spacing: theme.spacingMd

            Card {
                Layout.preferredWidth: 420
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: theme.spacingSm

                    Repeater {
                        model: transactionsVM.items

                        Button {
                            Layout.fillWidth: true
                            implicitHeight: 78
                            onClicked: transactionsVM.selectTransaction(modelData["id"])

                            contentItem: RowLayout {
                                spacing: theme.spacingMd

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 2

                                    Text {
                                        text: modelData["title"]
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

                                ColumnLayout {
                                    spacing: 2

                                    Text {
                                        text: modelData["amount_label"]
                                        color: theme.ink
                                        font.family: theme.bodyFont
                                        font.pixelSize: 12
                                    }

                                    Text {
                                        text: modelData["fiat_label"]
                                        color: theme.inkMuted
                                        font.family: theme.bodyFont
                                        font.pixelSize: 11
                                    }
                                }
                            }

                            background: Rectangle {
                                color: transactionsVM.selectedItem["id"] === modelData["id"] ? theme.cardAlt : "transparent"
                                radius: theme.radiusMd
                                border.color: theme.cardBorder
                                border.width: 1
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
                        text: transactionsVM.selectedItem["title"] || ""
                        color: theme.ink
                        font.family: theme.displayFont
                        font.pixelSize: 30
                        font.bold: true
                    }

                    Text {
                        text: transactionsVM.selectedItem["occurred_at_label"] || ""
                        color: theme.inkMuted
                        font.family: theme.bodyFont
                        font.pixelSize: 13
                    }

                    GridLayout {
                        Layout.fillWidth: true
                        columns: width > 760 ? 2 : 1
                        columnSpacing: theme.spacingMd
                        rowSpacing: theme.spacingMd

                        Repeater {
                            model: transactionsVM.selectedDetails

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

                    Card {
                        Layout.fillWidth: true
                        fillColor: theme.cardAlt

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: theme.spacingSm

                            Text {
                                text: "Description"
                                color: theme.inkMuted
                                font.family: theme.bodyFont
                                font.pixelSize: 12
                            }

                            Text {
                                Layout.fillWidth: true
                                wrapMode: Text.WordWrap
                                text: transactionsVM.selectedItem["description"] || ""
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
