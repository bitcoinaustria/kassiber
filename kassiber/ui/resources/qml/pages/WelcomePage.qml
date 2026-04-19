import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

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

                    Item {
                        width: 220
                        height: 220

                        Rectangle {
                            anchors.centerIn: parent
                            width: 182
                            height: 182
                            radius: 91
                            color: "transparent"
                            border.color: theme.warmShadow
                            border.width: 1
                        }

                        Rectangle {
                            anchors.centerIn: parent
                            width: 138
                            height: 138
                            radius: 69
                            color: "transparent"
                            border.color: theme.warmShadow
                            border.width: 1
                        }

                        Rectangle {
                            anchors.centerIn: parent
                            width: 88
                            height: 88
                            radius: 44
                            color: "transparent"
                            border.color: theme.warmShadow
                            border.width: 1
                        }

                        Item {
                            anchors.centerIn: parent
                            width: 48
                            height: 34

                            Rectangle {
                                anchors.fill: parent
                                color: "transparent"
                                border.color: theme.warmShadow
                                border.width: 2
                            }

                            Rectangle {
                                x: 0
                                y: 0
                                width: parent.width
                                height: 2
                                rotation: 35
                                transformOrigin: Item.Left
                                color: theme.warmShadow
                            }

                            Rectangle {
                                x: parent.width
                                y: 0
                                width: parent.width
                                height: 2
                                rotation: 145
                                transformOrigin: Item.Left
                                color: theme.warmShadow
                            }

                            Rectangle {
                                anchors.centerIn: parent
                                width: 14
                                height: 14
                                radius: 7
                                color: theme.accent
                            }
                        }

                        Text {
                            anchors.horizontalCenter: parent.horizontalCenter
                            y: 32
                            text: "BITCOIN · ACCOUNTING · AUSTRIA"
                            color: theme.warmShadow
                            font.family: theme.displayFont
                            font.pixelSize: 10
                            font.letterSpacing: 2.8
                        }

                        Text {
                            anchors.horizontalCenter: parent.horizontalCenter
                            y: parent.height - 46
                            text: "PRIVATE · LOCAL"
                            color: theme.warmShadow
                            font.family: theme.displayFont
                            font.pixelSize: 10
                            font.letterSpacing: 2.8
                        }
                    }

                    Text {
                        width: 280
                        horizontalAlignment: Text.AlignHCenter
                        text: dashboardVM.welcomeStampCaption
                        color: theme.warmShadow
                        font.family: theme.bodyFont
                        font.pixelSize: 12
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
                    font.family: theme.displayFont
                    font.pixelSize: 64
                    font.bold: false
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: dashboardVM.welcomeBody
                    color: Qt.rgba(0.18, 0.16, 0.14, 0.78)
                    font.family: theme.displayFont
                    font.pixelSize: 16
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
                            font.family: theme.bodyFont
                            font.pixelSize: 14
                            font.bold: true
                            font.letterSpacing: 2
                        }

                        TextField {
                            Layout.fillWidth: true
                            placeholderText: dashboardVM.welcomeNamePlaceholder
                            font.family: theme.displayFont
                            font.pixelSize: 16
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
                            font.family: theme.bodyFont
                            font.pixelSize: 14
                            font.bold: true
                            font.letterSpacing: 2
                        }

                        TextField {
                            Layout.fillWidth: true
                            text: dashboardVM.welcomeWorkspaceValue
                            font.family: theme.displayFont
                            font.pixelSize: 16
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
                        font.family: theme.bodyFont
                        font.pixelSize: 14
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
                                    font.family: theme.bodyFont
                                    font.pixelSize: 14
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
                        text: "  ->  Let's go"
                        onClicked: dashboardVM.selectPage(dashboardVM.hasProfile ? "overview" : "settings")
                        leftPadding: 18
                        rightPadding: 18
                        topPadding: 14
                        bottomPadding: 14

                        contentItem: Text {
                            text: parent.text
                            color: "#FFF8F1"
                            font.family: theme.bodyFont
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
                        font.family: theme.bodyFont
                        font.pixelSize: 13
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
