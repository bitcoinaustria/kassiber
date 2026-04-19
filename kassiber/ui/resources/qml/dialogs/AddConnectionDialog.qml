import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

Dialog {
    id: root
    title: "Add a connection"
    modal: true
    width: 720
    padding: 0
    standardButtons: Dialog.NoButton

    background: Rectangle {
        color: theme.paper
        border.color: theme.ink
        border.width: 1
    }

    header: Rectangle {
        height: 44
        color: "transparent"

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: theme.line
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 14
            anchors.rightMargin: 14

            Text {
                text: root.title
                color: theme.ink
                font.family: theme.serifFont
                font.pixelSize: 20
            }

            Item {
                Layout.fillWidth: true
            }

            ToolButton {
                text: "\u2715"
                onClicked: root.close()
                font.family: theme.monoFont
                font.pixelSize: 12
            }
        }
    }

    contentItem: ColumnLayout {
        spacing: 18

        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Kassiber is watch-only. Import via xpubs, descriptors, Lightning backends, or flat files without bringing private keys into the app."
            color: theme.ink2
            font.family: theme.sansFont
            font.pixelSize: 13
            lineHeight: 1.45
        }

        GridLayout {
            Layout.fillWidth: true
            columns: 2
            columnSpacing: 8
            rowSpacing: 8

            Repeater {
                model: [
                    { "label": "XPub", "detail": "Single-sig on-chain watch" },
                    { "label": "Descriptor", "detail": "Multisig wallet descriptor" },
                    { "label": "Core Lightning", "detail": "CLN node RPC" },
                    { "label": "LND", "detail": "Lightning Network Daemon" },
                    { "label": "NWC", "detail": "Nostr Wallet Connect" },
                    { "label": "CSV import", "detail": "One-shot from file" }
                ]

                delegate: Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 76
                    color: "transparent"
                    border.color: theme.line
                    border.width: 1

                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 14
                        spacing: 12

                        Rectangle {
                            width: 30
                            height: 30
                            color: theme.ink

                            Text {
                                anchors.centerIn: parent
                                text: modelData["label"].slice(0, 1)
                                color: theme.paper
                                font.family: theme.serifFont
                                font.pixelSize: 16
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2

                            Text {
                                text: modelData["label"]
                                color: theme.ink
                                font.family: theme.serifFont
                                font.pixelSize: 16
                            }

                            Text {
                                Layout.fillWidth: true
                                wrapMode: Text.WordWrap
                                text: modelData["detail"]
                                color: theme.ink3
                                font.family: theme.monoFont
                                font.pixelSize: 10
                            }
                        }

                        Text {
                            text: "\u2192"
                            color: theme.ink3
                            font.family: theme.monoFont
                            font.pixelSize: 14
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            color: theme.paper2
            border.color: theme.line
            border.width: 1
            implicitHeight: 72

            RowLayout {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 10

                Rectangle {
                    width: 14
                    height: 14
                    radius: 7
                    color: theme.accent
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Connection creation still routes through the CLI today. This modal mirrors the final IA so we can port the design system first and wire the flows next."
                    color: theme.ink2
                    font.family: theme.sansFont
                    font.pixelSize: 11
                    lineHeight: 1.45
                }
            }
        }
    }
}
