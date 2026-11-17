package com.example.mymusic

import android.net.Uri

/** Один трек из музыкальной библиотеки телефона. */
data class Track(
    val id: Long,
    val title: String,
    val artist: String,
    val album: String,
    val durationMs: Long,
    val uri: Uri,
    val artUri: Uri,
)
