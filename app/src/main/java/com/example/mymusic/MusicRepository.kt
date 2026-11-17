package com.example.mymusic

import android.content.ContentUris
import android.content.Context
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Читает музыку из системной медиатеки (MediaStore) — всё, что телефон уже нашёл сам. */
class MusicRepository(private val context: Context) {

    private val albumArtBase: Uri = Uri.parse("content://media/external/audio/albumart")

    suspend fun loadTracks(): List<Track> = withContext(Dispatchers.IO) {
        val collection = if (Build.VERSION.SDK_INT >= 29) {
            MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL)
        } else {
            MediaStore.Audio.Media.EXTERNAL_CONTENT_URI
        }

        val projection = arrayOf(
            MediaStore.Audio.Media._ID,
            MediaStore.Audio.Media.TITLE,
            MediaStore.Audio.Media.ARTIST,
            MediaStore.Audio.Media.ALBUM,
            MediaStore.Audio.Media.ALBUM_ID,
            MediaStore.Audio.Media.DURATION,
        )

        // IS_MUSIC отсекает рингтоны, уведомления и т.п.
        val selection = "${MediaStore.Audio.Media.IS_MUSIC} != 0"
        val sortOrder = "${MediaStore.Audio.Media.TITLE} COLLATE NOCASE ASC"

        val result = mutableListOf<Track>()
        context.contentResolver.query(collection, projection, selection, null, sortOrder)?.use { cursor ->
            val idCol = cursor.getColumnIndexOrThrow(MediaStore.Audio.Media._ID)
            val titleCol = cursor.getColumnIndexOrThrow(MediaStore.Audio.Media.TITLE)
            val artistCol = cursor.getColumnIndexOrThrow(MediaStore.Audio.Media.ARTIST)
            val albumCol = cursor.getColumnIndexOrThrow(MediaStore.Audio.Media.ALBUM)
            val albumIdCol = cursor.getColumnIndexOrThrow(MediaStore.Audio.Media.ALBUM_ID)
            val durationCol = cursor.getColumnIndexOrThrow(MediaStore.Audio.Media.DURATION)

            while (cursor.moveToNext()) {
                val id = cursor.getLong(idCol)
                val artist = cursor.getString(artistCol).cleanTag("Неизвестный исполнитель")
                val album = cursor.getString(albumCol).cleanTag("Неизвестный альбом")
                result += Track(
                    id = id,
                    title = cursor.getString(titleCol).cleanTag("Без названия"),
                    artist = artist,
                    album = album,
                    durationMs = cursor.getLong(durationCol),
                    uri = ContentUris.withAppendedId(collection, id),
                    artUri = ContentUris.withAppendedId(albumArtBase, cursor.getLong(albumIdCol)),
                )
            }
        }
        result
    }

    /** MediaStore отдаёт "<unknown>", если в файле нет тега. */
    private fun String?.cleanTag(fallback: String): String =
        if (this.isNullOrBlank() || this == "<unknown>") fallback else this
}
