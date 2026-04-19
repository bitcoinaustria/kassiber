import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"

Item {
    id: root
    property string selectedResidency: "AT"

    Rectangle {
        anchors.fill: parent
        color: theme.warmBg
    }

    Item {
        anchors.fill: parent
        anchors.margins: 28

        Repeater {
            model: Math.max(2, Math.floor(parent.width / 78))

            Rectangle {
                x: index * 78
                y: 0
                width: 1
                height: parent.height
                color: theme.warmGrid
                opacity: 0.85
            }
        }

        Repeater {
            model: Math.max(2, Math.floor(parent.height / 62))

            Rectangle {
                x: 0
                y: index * 62
                width: parent.width
                height: 1
                color: theme.warmGrid
                opacity: 0.85
            }
        }

        Rectangle {
            anchors.fill: parent
            color: "transparent"
            border.color: theme.warmGridStrong
            border.width: 1
        }
    }

    Item {
        id: cardFrame
        width: Math.min(root.width - 420, 980)
        height: Math.min(root.height - 260, 520)
        anchors.centerIn: parent
        anchors.verticalCenterOffset: 18

        Rectangle {
            x: 20
            y: 20
            width: parent.width
            height: parent.height
            color: theme.warmShadow
        }

        Rectangle {
            anchors.fill: parent
            color: theme.warmPaper
            border.color: theme.warmShadow
            border.width: 1
        }

        RowLayout {
            anchors.fill: parent
            anchors.margins: 42
            spacing: 42

            Item {
                Layout.fillHeight: true
                Layout.preferredWidth: 240

                Column {
                    anchors.centerIn: parent
                    spacing: 24

                    SealLockup {
                        size: 220
                        inkColor: theme.warmShadow
                        sealColor: theme.accent
                        ringText: "BITCOIN * ACCOUNTING * AUSTRIA * PRIVATE * LOCAL *"
                    }

                    Text {
                        width: 280
                        horizontalAlignment: Text.AlignHCenter
                        text: dashboardVM.welcomeStampCaption
                        color: theme.warmShadow
                        font.family: theme.monoFont
                        font.pixelSize: 11
                        font.letterSpacing: 2.8
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 14

                Item {
                    Layout.fillHeight: true
                }

                Text {
                    text: dashboardVM.welcomeTitle
                    color: theme.ink
                    font.family: theme.serifFont
                    font.pixelSize: 64
                    font.bold: false
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: dashboardVM.welcomeBody
                    color: Qt.rgba(0.18, 0.16, 0.14, 0.78)
                    font.family: theme.sansFont
                    font.pixelSize: 15
                    lineHeight: 1.35
                    Layout.maximumWidth: 520
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 14

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        Text {
                            text: "YOUR NAME"
                            color: theme.inkMuted
                            font.family: theme.monoFont
                            font.pixelSize: 13
                            font.bold: true
                            font.letterSpacing: 2
                        }

                        TextField {
                            Layout.fillWidth: true
                            placeholderText: dashboardVM.welcomeNamePlaceholder
                            font.family: theme.sansFont
                            font.pixelSize: 15
                            color: theme.ink
                            padding: 12
                            placeholderTextColor: Qt.rgba(0.28, 0.25, 0.22, 0.30)
                            background: Rectangle {
                                color: Qt.rgba(1, 1, 1, 0.10)
                                border.color: theme.warmBorder
                                border.width: 1
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        Text {
                            text: "WORKSPACE"
                            color: theme.inkMuted
                            font.family: theme.monoFont
                            font.pixelSize: 13
                            font.bold: true
                            font.letterSpacing: 2
                        }

                        TextField {
                            Layout.fillWidth: true
                            text: dashboardVM.welcomeWorkspaceValue
                            font.family: theme.sansFont
                            font.pixelSize: 15
                            color: theme.ink
                            padding: 12
                            background: Rectangle {
                                color: Qt.rgba(1, 1, 1, 0.10)
                                border.color: theme.warmBorder
                                border.width: 1
                            }
                        }
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    Text {
                        text: "TAX RESIDENCY"
                        color: theme.inkMuted
                        font.family: theme.monoFont
                        font.pixelSize: 13
                        font.bold: true
                        font.letterSpacing: 2
                    }

                    RowLayout {
                        spacing: 10

                        Repeater {
                            model: dashboardVM.welcomeResidencyOptions

                            Button {
                                property bool selected: root.selectedResidency === modelData["code"]
                                text: modelData["label"]
                                onClicked: root.selectedResidency = modelData["code"]
                                leftPadding: 16
                                rightPadding: 16
                                topPadding: 8
                                bottomPadding: 8

                                contentItem: Text {
                                    text: parent.text
                                    color: parent.selected ? theme.warmPaper : theme.inkMuted
                                    font.family: theme.monoFont
                                    font.pixelSize: 13
                                    font.bold: true
                                    horizontalAlignment: Text.AlignHCenter
                                    verticalAlignment: Text.AlignVCenter
                                }

                                background: Rectangle {
                                    radius: 18
                                    color: parent.selected ? theme.warmShadow : "transparent"
                                    border.color: parent.selected ? theme.warmShadow : theme.warmBorder
                                    border.width: 1
                                }
                            }
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16

                    Button {
                        text: "\u2192  Let's go"
                        onClicked: dashboardVM.selectPage(dashboardVM.hasProfile ? "overview" : "settings")
                        leftPadding: 18
                        rightPadding: 18
                        topPadding: 14
                        bottomPadding: 14

                        contentItem: Text {
                            text: parent.text
                            color: "#FFF8F1"
                            font.family: theme.sansFont
                            font.pixelSize: 16
                            font.bold: true
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }

                        background: Rectangle {
                            color: theme.warmAccent
                            border.color: theme.warmAccent
                            border.width: 1
                        }
                        implicitWidth: 190
                        implicitHeight: 56
                    }

                    Text {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        text: root.selectedResidency === "AT"
                            ? dashboardVM.welcomeResidencyNote
                            : "Tax defaults will follow the selected residency preset. Editable later."
                        color: theme.inkMuted
                        font.family: theme.monoFont
                        font.pixelSize: 12
                        Layout.maximumWidth: 360
                    }
                }

                Item {
                    Layout.fillHeight: true
                }
            }
        }
    }
}
