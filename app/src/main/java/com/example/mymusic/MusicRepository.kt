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

    /**
     * Фрагменты пути, по которым файл считается НЕ музыкой: мессенджеры и диктофоны.
     * Сравнение через SQL LIKE — без учёта регистра только для латиницы,
     * поэтому русские названия папок записаны в двух вариантах.
     * Чтобы вернуть что-то в список — просто удали строку.
     */
    private val excludedPathPatterns = listOf(
        // Мессенджеры
        "%Telegram%", "%org.telegram%",
        "%WhatsApp%", "%com.whatsapp%",
        "%Viber%",
        "%vkontakte%", "%/VK/%",
        "%/Signal/%",
        "%com.facebook.orca%",
        "%Skype%",
        "%Discord%",
        // Диктофоны и записи звонков
        "%/Recordings/%", "%/Recorder/%", "%/Recording/%",
        "%Voice Recorder%", "%VoiceRecorder%", "%Voice Notes%", "%Voice Memos%",
        "%sound_recorder%", "%call_rec%", "%CallRecord%",
        "%Диктофон%", "%диктофон%",
        "%Запись%", "%запись%", "%Записи%", "%записи%",
    )

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
        // Дальше убираем голосовые из мессенджеров и записи диктофона: по папкам и по флагу «запись».
        val data = MediaStore.Audio.Media.DATA
        val notInExcludedFolders = excludedPathPatterns.joinToString(" AND ") { "$data NOT LIKE ?" }
        val selection = buildString {
            append("${MediaStore.Audio.Media.IS_MUSIC} != 0")
            append(" AND ($data IS NULL OR ($notInExcludedFolders))")
            if (Build.VERSION.SDK_INT >= 31) {
                append(" AND COALESCE(${MediaStore.Audio.Media.IS_RECORDING}, 0) = 0")
            }
        }
        val selectionArgs = excludedPathPatterns.toTypedArray()
        val sortOrder = "${MediaStore.Audio.Media.TITLE} COLLATE NOCASE ASC"

        val result = mutableListOf<Track>()
        context.contentResolver.query(collection, projection, selection, selectionArgs, sortOrder)?.use { cursor ->
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
