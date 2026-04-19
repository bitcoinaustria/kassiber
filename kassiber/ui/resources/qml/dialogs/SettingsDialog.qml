import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

Dialog {
    id: root
    title: "Settings"
    modal: true
    width: 580
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

    contentItem: ScrollView {
        clip: true

        ColumnLayout {
            width: parent.width
            spacing: 18

            Repeater {
                model: [
                    { "title": "Privacy", "rows": settingsVM.privacyRows },
                    { "title": "App lock", "rows": settingsVM.lockRows },
                    { "title": "Data paths", "rows": settingsVM.cards }
                ]

                delegate: ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    Text {
                        text: modelData["title"]
                        color: theme.ink3
                        font.family: theme.monoFont
                        font.pixelSize: 10
                        font.bold: true
                        font.letterSpacing: 1.4
                    }

                    Repeater {
                        model: modelData["rows"]

                        delegate: Rectangle {
                            Layout.fillWidth: true
                            color: "transparent"
                            border.color: theme.line
                            border.width: 1
                            implicitHeight: bodyColumn.implicitHeight + 20

                            RowLayout {
                                anchors.fill: parent
                                anchors.margins: 10
                                spacing: 12

                                ColumnLayout {
                                    id: bodyColumn
                                    Layout.fillWidth: true
                                    spacing: 4

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData["label"] || ""
                                        color: theme.ink
                                        font.family: theme.sansFont
                                        font.pixelSize: 13
                                        wrapMode: Text.WordWrap
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData["detail"] || modelData["hint"] || modelData["value"] || ""
                                        color: theme.ink3
                                        font.family: modelData["value"] ? theme.monoFont : theme.sansFont
                                        font.pixelSize: 11
                                        wrapMode: Text.WrapAnywhere
                                    }
                                }

                                Rectangle {
                                    visible: modelData["enabled"] !== undefined
                                    width: 36
                                    height: 20
                                    color: modelData["enabled"] ? theme.ink : theme.line2

                                    Rectangle {
                                        width: 16
                                        height: 16
                                        y: 2
                                        x: modelData["enabled"] ? 18 : 2
                                        color: theme.paper2
                                    }
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
                    text: "Sync backends"
                    color: theme.ink3
                    font.family: theme.monoFont
                    font.pixelSize: 10
                    font.bold: true
                    font.letterSpacing: 1.4
                }

                Repeater {
                    model: settingsVM.backendRows

                    delegate: Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 44
                        color: "transparent"
                        border.color: theme.line
                        border.width: 1

                        RowLayout {
                            anchors.fill: parent
                            anchors.margins: 10
                            spacing: 10

                            Rectangle {
                                width: 6
                                height: 6
                                radius: 3
                                color: modelData["status"] === "active" ? theme.ok : theme.ink3
                            }

                            Text {
                                text: modelData["label"] || ""
                                color: theme.ink
                                font.family: theme.monoFont
                                font.pixelSize: 12
                            }

                            Text {
                                Layout.fillWidth: true
                                text: modelData["value"] || ""
                                color: theme.ink3
                                font.family: theme.monoFont
                                font.pixelSize: 10
                                elide: Text.ElideMiddle
                            }
                        }
                    }
                }
            }
        }
    }
}
