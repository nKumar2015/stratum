import QtQuick
import QtQuick.Layouts

import "../globals"

Rectangle {
    id: root

    property bool panelVisible: false
    property string musicStatus: "Unknown"
    property string musicTitle: "Nothing playing"
    property string musicArtist: "N/A"
    property string musicAlbum: "N/A"
    property string musicPosition: "00:00"
    property string musicLength: "00:00"
    property string musicArtUrl: ""

    signal previousRequested()
    signal playPauseRequested()
    signal nextRequested()

    implicitHeight: musicColumn.implicitHeight + 20
    color: Theme.palette.bgMain
    radius: 10
    border.width: 1
    border.color: Theme.palette.borderInactive

    ColumnLayout {
        id: musicColumn
        anchors.fill: parent
        anchors.margins: 10
        spacing: 6

        Text {
            text: "Now Playing"
            color: Theme.palette.textMain
            font.family: Theme.palette.font
            font.pixelSize: 14
            font.bold: true
            horizontalAlignment: Text.AlignHCenter
            Layout.fillWidth: true
        }

        Rectangle {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: 200
            Layout.preferredHeight: 200
            radius: 10
            color: Theme.palette.bgWidget
            border.width: 1
            border.color: Theme.palette.borderInactive
            clip: true

            Image {
                anchors.fill: parent
                source: root.musicArtUrl
                fillMode: Image.PreserveAspectCrop
                visible: root.musicArtUrl.length > 0
                smooth: true
                asynchronous: true
            }

            Text {
                anchors.centerIn: parent
                text: "󰎆"
                visible: root.musicArtUrl.length === 0
                color: Theme.palette.textMain
                font.family: Theme.palette.font
                font.pixelSize: 36
            }
        }

        Item {
            id: musicTitleClip
            Layout.fillWidth: true
            Layout.minimumHeight: musicTitleTextA.implicitHeight
            implicitHeight: musicTitleTextA.implicitHeight
            clip: true

            property int marqueeGap: 24
            property real scrollSpeed: 42
            property bool titleOverflow: musicTitleTextA.implicitWidth > width
            property real loopSpan: musicTitleTextA.implicitWidth + marqueeGap
            property real tickerOffset: 0

            Text {
                id: musicTitleTextA
                text: root.musicTitle
                color: Theme.palette.textMain
                font.family: Theme.palette.font
                font.pixelSize: 12
                font.bold: true
                elide: Text.ElideNone
                wrapMode: Text.NoWrap
                anchors.verticalCenter: parent.verticalCenter
                x: musicTitleClip.titleOverflow ? musicTitleClip.tickerOffset : Math.round((musicTitleClip.width - implicitWidth) / 2)
            }

            Text {
                id: musicTitleTextB
                text: root.musicTitle
                color: Theme.palette.textMain
                font.family: Theme.palette.font
                font.pixelSize: 12
                font.bold: true
                elide: Text.ElideNone
                wrapMode: Text.NoWrap
                visible: musicTitleClip.titleOverflow
                anchors.verticalCenter: parent.verticalCenter
                x: musicTitleClip.tickerOffset + musicTitleClip.loopSpan
            }

            NumberAnimation {
                id: musicTitleMarquee
                target: musicTitleClip
                property: "tickerOffset"
                from: 0
                to: -musicTitleClip.loopSpan
                duration: Math.max(1, Math.round((musicTitleClip.loopSpan / musicTitleClip.scrollSpeed) * 1000))
                easing.type: Easing.Linear
                running: root.panelVisible && musicTitleClip.titleOverflow
                loops: Animation.Infinite

                onRunningChanged: {
                    if (!running)
                        musicTitleClip.tickerOffset = 0;
                }
            }

            onTitleOverflowChanged: {
                if (!titleOverflow)
                    tickerOffset = 0;
            }
        }

        Text {
            text: root.musicArtist
            color: Theme.palette.secondary
            font.family: Theme.palette.font
            font.pixelSize: 11
            elide: Text.ElideRight
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
        }

        Text {
            text: root.musicAlbum
            color: Theme.palette.tertiary
            font.family: Theme.palette.font
            font.pixelSize: 10
            elide: Text.ElideRight
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
        }

        Text {
            text: root.musicPosition + " / " + root.musicLength
            color: Theme.palette.textMain
            font.family: Theme.palette.font
            font.pixelSize: 10
            horizontalAlignment: Text.AlignHCenter
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignHCenter
            spacing: 10

            Rectangle {
                Layout.preferredWidth: 56
                Layout.preferredHeight: 40
                radius: 8
                color: prevButton.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget
                border.color: prevButton.containsMouse ? Theme.palette.borderActive: Theme.palette.borderInactive
                border.width: 1
                
                Text {
                    anchors.centerIn: parent
                    text: ""
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 16
                }

                MouseArea {
                    id: prevButton
                    hoverEnabled: true
                    anchors.fill: parent
                    onClicked: root.previousRequested()
                }
            }

            Rectangle {
                Layout.preferredWidth: 72
                Layout.preferredHeight: 40
                radius: 8
                color: pausePlayButton.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget
                border.color: pausePlayButton.containsMouse ? Theme.palette.borderActive: Theme.palette.borderInactive
                border.width: 1
                
                Text {
                    anchors.centerIn: parent
                    text: root.musicStatus === "Playing" ? "" : ""
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 16
                }

                MouseArea {
                    id: pausePlayButton
                    hoverEnabled: true
                    anchors.fill: parent
                    onClicked: root.playPauseRequested()
                }
            }

            Rectangle {
                Layout.preferredWidth: 56
                Layout.preferredHeight: 40
                radius: 8
                color: nextButton.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget
                border.color: nextButton.containsMouse ? Theme.palette.borderActive: Theme.palette.borderInactive
                border.width: 1

                Text {
                    anchors.centerIn: parent
                    text: ""
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 16
                }

                MouseArea {
                    id: nextButton
                    hoverEnabled: true
                    anchors.fill: parent
                    onClicked: root.nextRequested()
                }
            }
        }
    }
}
