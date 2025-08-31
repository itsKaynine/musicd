const app = document.getElementById('app');
const trackNameEl = document.getElementById('track-name');
const trackNameDupEl = document.getElementById('track-name-dup');
const trackNameMarqueeEl = document.getElementById('track-name-marquee');
const prevBtnEl = document.getElementById('prev');
const nextBtnEl = document.getElementById('next');
const playBtnEl = document.getElementById('play');
const seekEl = document.getElementById('seek');
const volumeEl = document.getElementById('volume');
const currentTimeEl = document.getElementById('current');
const durationEl = document.getElementById('duration');
const playlistsPlaylistNameEl = document.getElementById('playlists-playlist-name');
const tracksPlaylistNameEl = document.getElementById('tracks-playlist-name');
const refreshTracksBtnEl = document.getElementById('refresh-tracks');
const refreshPlaylistsBtnEl = document.getElementById('refresh-playlists');
const devStatusEl = document.getElementById('dev-status');
const devStateEl = document.getElementById('dev-state');
const devJobsEl = document.getElementById('dev-jobs');
const publishAlertEl = document.getElementById('publish-alert');
const publishNameEl = document.getElementById('publish-name');
const publishSourcesEl = document.getElementById('publish-sources');
const publishBtnEl = document.getElementById('publish-btn');

let statusRef = null;

let state = {
  isScrubbing: false,
  isChangingVolume: false,
  isPaused: false,
  currentPlaylistId: undefined,
  currentTrackIndex: undefined,
  currentTrackName: undefined,
};

const formatTime = (secs) => {
  const min = Math.floor(secs / 60) || 0;
  const sec = Math.floor(secs % 60) || 0;
  return `${min}:${sec.toString().padStart(2, '0')}`;
};

function escapeHtml(html) {
  const div = document.createElement('div');
  div.textContent = html;
  return div.innerHTML;
}

function toast(message, variant = 'primary', icon = 'info-circle', duration = 3000) {
  const alert = Object.assign(document.createElement('sl-alert'), {
    variant,
    closable: true,
    countdown: 'rtl',
    duration,
    innerHTML: `
      <sl-icon name="${icon}" slot="icon"></sl-icon>
      ${escapeHtml(message)}
    `
  });

  document.body.append(alert);
  return alert.toast();
}

function resetTrackNameMarquee() {
  const container = trackNameMarqueeEl.parentElement;

  container.classList.remove('scrolling');
  trackNameDupEl.style.display = 'none';
  trackNameMarqueeEl.style.animation = 'none';
}

function setupTrackNameMarquee() {
  const container = trackNameMarqueeEl.parentElement;

  const textWidth = trackNameEl.scrollWidth;
  const containerWidth = container.clientWidth;

  if (textWidth > containerWidth) {
    container.classList.add('scrolling');
    trackNameDupEl.style.display = '';

    const gap = 50;
    const distance = textWidth + gap;
    trackNameMarqueeEl.style.setProperty('--shift', -distance + 'px');

    const speedPixelsPerSecond = 30;
    const duration = distance / speedPixelsPerSecond;

    trackNameMarqueeEl.style.animation = 'scroll-text linear infinite';
    trackNameMarqueeEl.style.animationDuration = duration + 's';
    trackNameMarqueeEl.style.animationPlayState = 'running';
  } else {
    resetTrackNameMarquee();
  }
}

async function refreshStatus() {
  const status = await fetch('/status').then((r) => r.json());

  // Update dev status and state
  devStatusEl.textContent = JSON.stringify(status, null, 2);
  devStateEl.textContent = JSON.stringify(state, null, 2);

  // Only on first load
  if (!statusRef) {
    // Set initial state
    state.isPaused = status.is_paused;
    state.currentPlaylistId = status.playlist_id;
    state.currentTrackName = status.current_track;
    state.currentTrackIndex = status.current_index;

    // Only render new value if not scrubbing
    if (!state.isScrubbing && status.current_pos !== null && status.current_pos !== undefined) {
      renderSeekPosition(Number(`${status.current_pos.secs}.${status.current_pos.nanos}`));
    }

    if (status.current_pos) {
      renderCurrentTime(status.current_pos.secs);
    }

    // Only render new value if not changing value
    if (!state.isChangingVolume && status.volume !== null && status.volume !== undefined) {
      renderVolume(status.volume);
    }

    if (status.total_duration) {
      renderSeekTotalDuration(Number(`${status.total_duration.secs}.${status.total_duration.nanos}`));
      renderTotalDuration(status.total_duration.secs);
    }

    renderCurrentTrack(status.current_track);
    renderPlaylistName(status.playlist_name);
    renderPlayButton(status.is_paused);
  }

  statusRef = status;
}

function renderPlayButton() {
  if (state.isPaused) {
    playBtnEl.innerHTML = '<sl-icon name="play-fill"></sl-icon>';
  } else {
    playBtnEl.innerHTML = '<sl-icon name="pause-fill"></sl-icon>';
  }
}

function renderSeekPosition(secs) {
  seekEl.value = secs;
}

function renderCurrentTime(secs) {
  currentTimeEl.textContent = formatTime(secs);
}

function renderSeekTotalDuration(secs) {
  seekEl.max = secs;
}

function renderTotalDuration(secs) {
  durationEl.textContent = formatTime(secs);
}

function renderVolume(value) {
  volumeEl.value = value;
}

function renderCurrentTrack(trackName) {
  const cleanTrackName = (trackName ?? '').replace(/\.[^/.]+$/, '');
  trackNameEl.textContent = cleanTrackName;
  trackNameDupEl.textContent = cleanTrackName;

  // NOTE: Wait for the text to update
  resetTrackNameMarquee();
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      setupTrackNameMarquee();
    });
  });
}

function renderPlaylistName(playlistName) {
  const text = `Playlist: ${playlistName ?? '(not selected)'}`;
  tracksPlaylistNameEl.textContent = text;
  playlistsPlaylistNameEl.textContent = text;
}

async function refreshJobs() {
  const jobs = await fetch('/jobs').then((r) => r.json());

  // Update dev jobs
  devJobsEl.textContent = JSON.stringify(jobs, null, 2);
}

async function selectTrack(index) {
  if (state.currentTrackIndex === index) {
    return;
  }

  await fetch(`/control/track/${index}`, { method: 'POST' });
  refreshPlaylist();
}

async function setPlaylist(playlistId, mode) {
  await fetch(`/control/playlist/${playlistId}`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ mode }),
  });
}

function renderPlaylists(playlists) {
  document.getElementById('playlists-tbody').innerHTML = playlists.map((p, i) =>
    `<tr
      data-src="${p.meta.id}"
      onclick="setPlaylist('${p.meta.id}', 'skip')"
      class="playlist-tr ${state.currentPlaylistId === p.meta.id ? 'current-playlist' : ''}"
    >
      <td>${i + 1}</td>
      <td>${p.meta.name}</td>
      <td class="playlist-actions">
        <sl-tooltip content="Queue Playlist">
          <sl-button
            size="small"
            circle
            variant="default"
            onclick="event.stopPropagation(); setPlaylist('${p.meta.id}', 'queue');"
          >
            <sl-icon name="clock"></sl-icon>
          </sl-button>
        </sl-tooltip>
        <sl-dropdown onclick="event.stopPropagation();">
          <sl-button slot="trigger" size="small" circle variant="default">
            <sl-icon name="three-dots" style="vertical-align: middle;"></sl-icon>
          </sl-button>
          <sl-menu class="playlist-menu">
            <sl-menu-item onclick="navigator.clipboard.writeText('${p.meta.id}'); toast('Copied!');">
              <sl-icon slot="prefix" name="copy" class="playlist-menu-item-icon"></sl-icon>
              <span class="playlist-menu-item-text">Copy ID</span>
            </sl-menu-item>
          </sl-menu>
        </sl-dropdown>
      </td>
    </tr>`
  ).join('');
}

function renderTracks(playlist) {
  if (!state.currentPlaylistId) {
    return;
  }

  document.getElementById('tracks-tbody').innerHTML = playlist.meta.tracks.map((trackName, i) =>
    `<tr
      data-src="${i}"
      class="track-tr ${state.currentTrackName === trackName ? 'current-track' : ''}"
      onclick="selectTrack(${i})"
    >
      <td>${i + 1}</td>
      <td>${trackName}</td>
    </tr>`
  ).join('');
}

async function refreshPlaylist() {
  const playlist = await fetch('/playlists').then((r) => r.json());
  renderPlaylists(playlist);

  const currentPlaylist = playlist.find((p) => state.currentPlaylistId == p.meta.id);
  if (currentPlaylist) {
    renderTracks(currentPlaylist);
  }
}

seekEl.tooltipFormatter = (value) => {
  return formatTime(value);
};

volumeEl.tooltipFormatter = (value) => {
  const percent = Math.floor(value * 100);
  return `${percent}%`;
};

// Scrub track
seekEl.addEventListener('sl-input', () => {
  state.isScrubbing = true;
});

seekEl.addEventListener('sl-change', async () => {
  setTimeout(() => {
    state.isScrubbing = false;
  }, 0);

  await fetch('/control/seek', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ secs: seekEl.value }),
  });

  setTimeout(() => {
    state.isScrubbing = false;
  }, 1000);
});

// Volume control
seekEl.addEventListener('sl-input', () => {
  state.isChangingVolume = true;
});

volumeEl.addEventListener('sl-change', () => {
  setTimeout(() => {
    state.isChangingVolume = false;
  }, 0);

  fetch('/control/volume', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ value: volumeEl.value }),
  });

  setTimeout(() => {
    state.isChangingVolume = false;
  }, 1000);
});

// Skip buttons
prevBtnEl.addEventListener('click', async () => {
  await fetch('/control/prev', { method: 'POST' });
});
nextBtnEl.addEventListener('click', async () => {
  await fetch('/control/next', { method: 'POST' });
});

// Refresh buttons
refreshTracksBtnEl.addEventListener('click', () => {
  refreshPlaylist();
});
refreshPlaylistsBtnEl.addEventListener('click', () => {
  refreshPlaylist();
});

// Play / pause toggle
playBtnEl.addEventListener('click', async () => {
  if (state.isPaused) {
    playBtnEl.innerHTML = '<sl-icon name="pause-fill"></sl-icon>';
    await fetch('/control/play', { method: 'POST' });
  } else {
    playBtnEl.innerHTML = '<sl-icon name="play-fill"></sl-icon>';
    await fetch('/control/pause', { method: 'POST' });
  }
});

// Publish
publishBtnEl.addEventListener('click', async () => {
  const name = publishNameEl.value;
  const sources = publishSourcesEl.value
    .split('\n')
    .map((x) => x.trim())
    .filter((x) => x);

  publishBtnEl.disabled = true;
  await fetch('/publish', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ name, source_urls: sources }),
  });
  publishBtnEl.disabled = false;
  publishAlertEl.show();
});

async function init() {
  setupTrackNameMarquee();

  await refreshStatus();
  refreshPlaylist();
  refreshJobs();

  setInterval(refreshStatus, 1500);
}

document.addEventListener("DOMContentLoaded", async (event) => {
  await init();
  app.classList.add('sl-ready');
});

const ws = new ReconnectingWebSocket('/ws');

ws.on('open', () => {
  console.log('Connected!');
});

ws.on('close', () => {
  console.log('Disconnected, will retry…');
});

ws.on('error', (err) => {
  console.error('WebSocket error:', err);
});

ws.on('message', (event) => {
  const json = JSON.parse(event.data);
  const { type, payload } = json;

  switch (type) {
    case 'PLAYED': {
      state.isPaused = false;

      renderPlayButton();
      break;
    }
    case 'PAUSED': {
      state.isPaused = true;

      renderPlayButton();
      break;
    }
    case 'TRACK_CHANGED': {
      const { idx, name } = payload;

      state.currentTrackIndex = idx;
      state.currentTrackName = name;

      refreshPlaylist();
      resetTrackNameMarquee();
      renderCurrentTrack(name);
      break;
    }
    case 'TRACK_DURATION_CHANGED': {
      const { duration } = payload;

      renderSeekTotalDuration(Number(`${duration.secs}.${duration.nanos}`));
      renderTotalDuration(duration.secs);
      break;
    }
    case 'PLAYLIST_CHANGED': {
      const { id, name } = payload;

      state.currentPlaylistId = id;

      renderPlaylistName(name);
      refreshPlaylist();
      break;
    }
    case 'PLAYLIST_PUBLISHED': {
      const { name } = payload;

      toast(`Playlist ${name} successfully published`);

      refreshPlaylist();
      break;
    }
    case 'SEEK_POSITION_CHANGED': {
      const { duration } = payload;

      if (!state.isScrubbing) {
        renderSeekPosition(Number(`${duration.secs}.${duration.nanos}`));
      }
      renderCurrentTime(duration.secs);
      break;
    }
    case 'VOLUME_CHANGED': {
      const { value } = payload;

      if (!state.isChangingVolume) {
        renderVolume(value);
      }
      break;
    }
    case 'JOBS_UPDATED': {
      refreshJobs();
      break;
    }
    case 'RUNNING_JOB': {
      const { id } = payload;

      toast(`Running job (id: ${id})`);
      break;
    }
  }
});
