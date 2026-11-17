package com.example.mymusic

import android.app.Application
import android.content.ComponentName
import androidx.core.content.ContextCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Всё, что нужно экрану, чтобы нарисовать плеер. */
data class PlayerUiState(
    val tracks: List<Track> = emptyList(),
    val loading: Boolean = false,
    val currentMediaId: String? = null,
    val currentTrack: Track? = null,
    val isPlaying: Boolean = false,
    val positionMs: Long = 0L,
    val durationMs: Long = 0L,
    val shuffle: Boolean = false,
    val repeatMode: Int = Player.REPEAT_MODE_OFF,
)

class PlayerViewModel(app: Application) : AndroidViewModel(app) {

    private val repository = MusicRepository(app)

    private val _state = MutableStateFlow(PlayerUiState())
    val state: StateFlow<PlayerUiState> = _state.asStateFlow()

    private var controller: MediaController? = null
    private var trackById: Map<String, Track> = emptyMap()

    private val listener = object : Player.Listener {
        override fun onEvents(player: Player, events: Player.Events) = sync()
    }

    init {
        // Подключаемся к PlaybackService — он сам запустится, если ещё не работает.
        val token = SessionToken(app, ComponentName(app, PlaybackService::class.java))
        val future = MediaController.Builder(app, token).buildAsync()
        future.addListener({
            controller = runCatching { future.get() }.getOrNull()
            controller?.addListener(listener)
            sync()
        }, ContextCompat.getMainExecutor(app))

        // Позиция трека не присылает событий, поэтому раз в полсекунды спрашиваем сами.
        viewModelScope.launch {
            while (true) {
                delay(500)
                if (controller?.isPlaying == true) sync()
            }
        }
    }

    fun loadTracks() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true) }
            val tracks = runCatching { repository.loadTracks() }.getOrDefault(emptyList())
            trackById = tracks.associateBy { it.id.toString() }
            _state.update { it.copy(tracks = tracks, loading = false) }
            sync()
        }
    }

    /** Играет [track], а очередью становится весь переданный список (например, результаты поиска). */
    fun play(track: Track, queue: List<Track>) {
        val c = controller ?: return
        val index = queue.indexOfFirst { it.id == track.id }.coerceAtLeast(0)
        c.setMediaItems(queue.map { it.toMediaItem() }, index, 0L)
        c.prepare()
        c.play()
    }

    fun togglePlayPause() {
        val c = controller ?: return
        if (c.isPlaying) {
            c.pause()
        } else {
            if (c.playbackState == Player.STATE_IDLE) c.prepare()
            if (c.playbackState == Player.STATE_ENDED) c.seekTo(0, 0L)
            c.play()
        }
    }

    fun next() {
        controller?.seekToNextMediaItem()
    }

    fun previous() {
        controller?.seekToPrevious()
    }

    fun seekTo(positionMs: Long) {
        controller?.seekTo(positionMs)
        _state.update { it.copy(positionMs = positionMs) }
    }

    fun toggleShuffle() {
        val c = controller ?: return
        c.shuffleModeEnabled = !c.shuffleModeEnabled
    }

    /** Повтор: выключен -> весь список -> один трек -> выключен. */
    fun cycleRepeat() {
        val c = controller ?: return
        c.repeatMode = when (c.repeatMode) {
            Player.REPEAT_MODE_OFF -> Player.REPEAT_MODE_ALL
            Player.REPEAT_MODE_ALL -> Player.REPEAT_MODE_ONE
            else -> Player.REPEAT_MODE_OFF
        }
    }

    private fun sync() {
        val c = controller ?: return
        val mediaId = c.currentMediaItem?.mediaId
        _state.update {
            it.copy(
                currentMediaId = mediaId,
                currentTrack = mediaId?.let { id -> trackById[id] },
                isPlaying = c.isPlaying,
                positionMs = c.currentPosition.coerceAtLeast(0L),
                durationMs = c.duration.takeIf { d -> d != C.TIME_UNSET } ?: 0L,
                shuffle = c.shuffleModeEnabled,
                repeatMode = c.repeatMode,
            )
        }
    }

    override fun onCleared() {
        controller?.removeListener(listener)
        controller?.release()
        controller = null
        super.onCleared()
    }
}

private fun Track.toMediaItem(): MediaItem =
    MediaItem.Builder()
        .setMediaId(id.toString())
        .setUri(uri)
        .setMediaMetadata(
            MediaMetadata.Builder()
                .setTitle(title)
                .setArtist(artist)
                .setAlbumTitle(album)
                .setArtworkUri(artUri)
                .build()
        )
        .build()
