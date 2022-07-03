window.player = { uid: NaN };
window.elem = {
  info: document.getElementById("miscinfo"),
  identform: document.getElementById("identform"),
  statusline: document.getElementsByClassName("statusline")[0],
  bcont: document.getElementById("board-container"),
  board: document.getElementById("board"),
  cursor_frame: document.getElementById("cursor-frame"),
  volslider: document.getElementById("volslider")
};

window.queued_pos = undefined;
window.assets = {
  audio: {
    explosion: registerMediaElem(new Audio("../explosion.opus"))
  }
};

function registerMediaElem(melm) {
  let record = { data: melm, loaded: false };
  melm.addEventListener("canplaythrough", e => {
    record.loaded = true;
  });
  return record;
}

document.addEventListener('roomloading', function() {
  //window.elem.info.innerHTML = "loading...";
  // TODO/FIXME: this should check all the assets, once we have more than one
  if (assets.audio.explosion.loaded) {
    room.socket = connect(); // drive the game with socket events from then on
  } else {
    setTimeout(function() { document.dispatchEvent(new Event('roomloading')) }, 500);
  }
});

window.room = {
  name: undefined,
  bconf: { w: NaN, h: NaN, tile_w: NaN, tile_h: NaN, mine_ratio: undefined },
  board: {},
  cbounds: {},
  socket: undefined,
  last_packet: undefined,
  identity: JSON.parse(localStorage.getItem("identity")),
  cursors: new Map(),
};


if (room.identity == null) {
  elem.statusline.style.display = "none";
  elem.identform.style.display = "initial";
} else {
  join();
}

function join() {
  if (room.identity == null) {
    room.identity = {};
    room.identity.name = document.getElementById("name-in").value;
    room.identity.clr = document.getElementById("clr-in").value;
    localStorage.setItem("identity", JSON.stringify(room.identity));
  }
  elem.identform.style.display = "none";
  elem.statusline.style.display = "flex";
  document.dispatchEvent(new Event('roomloading'));
}
function clear_ident() {
  localStorage.removeItem("identity");
  document.location.reload();
}

function connect() {
  let wsproto = (window.location.protocol == "https:")? "wss:": "ws:";
  let s = new WebSocket(`${wsproto}//${location.hostname}:${location.port}${location.pathname}/ws`);
  s.onopen = function() {
    s.send(`register ${room.identity.name} ${room.identity.clr}`);
  }
  s.onmessage = function(e) {
    room.last_packet = e;
    let d = e.data;
    if (typeof d == "object") {
      d.arrayBuffer().then(acceptBoard);
      elem.info.onclick = undefined;
      elem.info.innerHTML = `${room.name} (${room.bconf.w}x${room.bconf.h}) >> Running, ${room.bconf.mine_ratio} tiles are mines`;
    } else if (typeof e.data == "string") {
      let fields = d.split(" ");
      switch (fields[0]) {
        case "pos": {
          let posdata = JSON.parse(fields[1]);
          posdata.forEach(pdat => {
            let oid = Number(pdat[0]);
            let x = pdat[1][0];
            let y = pdat[1][1];
            let curs = room.cursors.get(oid);
            if (oid != player.uid) {
              if (curs != undefined) {
                movCursor(curs, x, y);
              } else {
                console.log("livepos sys incoherent");
              }
            }
          });
        } break;
        case "players": {
          let pdata = JSON.parse(fields[1]);
          console.log(pdata);
          pdata.forEach(p => {
            let oid = Number(p[0]);
            let name = p[1];
            let clr = p[2];
            console.log(oid, name, clr);
            if (!room.cursors.has(oid)) {
              createCursor(oid, name, clr);
            }
          });
        } break;
        case "regack": {
          room.name = fields[1];
          name = fields[2];
          player.uid = Number(fields[3]);
          let dims = fields[4].split("x");
          room.bconf.w = Number(dims[0]);
          room.bconf.h = Number(dims[1]);
          room.bconf.mine_ratio = fields[5];
          createCursor(player.uid, name, room.identity.clr);
        } break;
        case "win": {
          elem.info.innerHTML = "You win! Click here to play again.";
          elem.info.onclick = e => { s.send("reset") };
        } break;
        case "lose": {
          let badone = fields[1];
          elem.info.innerHTML = `You lost, ${badone} was blown up. Click here to retry.`;
          elem.info.onclick = e => { s.send("reset") };
          assets.audio.explosion.data.play();
        } break;
        case "logoff": {
          let oid = Number(fields[1]);
          room.cursors.get(oid).elem.remove();
          room.cursors.get(oid).selwin.remove();
          room.cursors.delete(oid);
        } break;
      }
    }
  }
  s.onerror = function(e) { elem.info.innerHTML += `<br>Connection error: ${e}`; }
  s.onclose = function(e) { elem.info.innerHTML = "Connection closed"; }
  return s;
}

function acceptBoard(data) {
  let dataarr = new Uint8Array(data);
  let vals = fflate.inflateSync(dataarr);
  room.board = vals.reduce((s,c) => {
    let v = String.fromCodePoint(c);
    if (v == ' ') {
      s = s + "&nbsp";
    } else {
      s = s + v;
    }
    return s;
  }, "");
  let last = room.board[0];
  let last_idx = 0;
  let split_board = [];
  for (let i = 1; i < room.board.length+1; i++) {
    let cur = room.board[i];
    let gamechars = /^[CFO# 1-8]+$/;
    if ((cur != last && gamechars.test(cur)) || cur == undefined) {
      let txt = room.board.substr(last_idx, i-last_idx);
      switch(txt[0]) {
        case 'O':
          txt = `<span style="color:red;">${txt}</span>`;
          break;
        case 'C':
          txt = `<span style="color:green;">${txt}</span>`;
          break;
        case 'F':
          txt = `<span style="color:yellow;">${txt}</span>`;
          break;

        case '1': txt = `<span style="color:#0100FB;">${txt}</span>`; break;
        case '2': txt = `<span style="color:#027F01;">${txt}</span>`; break;
        case '3': txt = `<span style="color:#FD0100;">${txt}</span>`; break;
        case '4': txt = `<span style="color:#01017B;">${txt}</span>`; break;
        case '5': txt = `<span style="color:#7D0302;">${txt}</span>`; break;
        case '6': txt = `<span style="color:#00807F;">${txt}</span>`; break;

        default: txt = `<span style="color:white;">${txt}</span>`; break;
      }
      split_board.push(txt);
      last_idx = i;
    }
    last = room.board[i];
  }
  elem.board.innerHTML = split_board.join("");
  room.cbounds = getBoardBounds();
}

function createCursor(id, name, clr) {
  // shit doesn't line up
  let cursor = document.createElement("div");
  cursor.style.position = "absolute";
  let nametag = document.createElement("p");
  nametag.innerHTML = name;
  nametag.classList.add('cursor-name');
  let selection_window = document.createElement("div");
  selection_window.style.backgroundColor = clr + "a0";
  selection_window.style.position = "absolute";
  selection_window.classList.add('cursor');
  cursor.appendChild(nametag);
  cursor.classList.add('cursor');
  cursor.style.color = clr;
  elem.cursor_frame.append(cursor);
  elem.cursor_frame.append(selection_window);
  let c = { name: name, elem: cursor, selwin: selection_window };
  if (id == window.player.uid) {
    document.addEventListener('mousemove', e => {
      let bcoords = pageToBoardPx(e.pageX, e.pageY);
      movCursor(c, bcoords[0], bcoords[1]);
      window.queued_pos = bcoords;
    },
      false);
  }
  room.cursors.set(id, {name: name, elem: cursor, selwin: selection_window});
  return cursor;
}

function pageToBoardPx(x,y) {
  return [Math.floor(x - room.cbounds.ox), Math.floor(y - room.cbounds.oy)];
}

function movCursor(c, bx, by) {
  c.elem.style.left = (room.cbounds.ox + bx) + 'px';
  c.elem.style.top = (room.cbounds.oy + by) + 'px';
  movSelWin(c.selwin, bx, by);
}
function movSelWin(win, bx, by) {
  let tpos = tilepos(bx,by);
  if (tpos.x > (room.bconf.w - 1) || tpos.x < 0 || tpos.y > (room.bconf.h - 1) || tpos.y < 0) {
    win.style.display = "none";
  } else {
    win.style.display = "";
  }
  win.style.left = (tpos.x * room.bconf.tile_w) + 'px';
  win.style.top  = (tpos.y * room.bconf.tile_h) + 'px';
  win.style.width = room.bconf.tile_w + 'px';
  win.style.height = room.bconf.tile_h + 'px';
}
function getBoardBounds() {
  let a = elem.bcont.getBoundingClientRect();
  let b = elem.board.getBoundingClientRect();
  room.bconf.tile_w = b.width / room.bconf.w;
  room.bconf.tile_h = 48;
  return {
    ox: b.x + window.scrollX,
    oy: a.y + window.scrollY,
    w: b.width,
    h: a.height
  };
}
window.onresize = () => {
  room.cbounds = getBoardBounds();
}

elem.bcont.onclick = function(e) {
  let bcoords = pageToBoardPx(e.pageX, e.pageY);
  let tpos = tilepos(bcoords[0], bcoords[1]);
  let cmd = `reveal ${tpos.x} ${tpos.y}`;
  room.socket.send(cmd);
}
elem.bcont.oncontextmenu = function(e) {
  let bcoords = pageToBoardPx(e.pageX, e.pageY);
  let tpos = tilepos(bcoords[0], bcoords[1]);
  let cmd = `flag ${tpos.x} ${tpos.y}`;
  room.socket.send(cmd);
  return false;
}
// these are board-px coords
function tilepos(bx,by) {
  let b = room.cbounds; // we can assume it is already computed by earlier aux calls
  let tilex = Math.floor(room.bconf.w * bx/b.w);
  let tiley = Math.floor(room.bconf.h * by/b.h);
  return { x: tilex, y: tiley };
}

function volChanged() {
  let newVol = elem.volslider.value;
  localStorage.setItem("audioVolume", JSON.stringify(newVol));
  for (i of Object.keys(assets.audio)) {
    assets.audio[i].data.volume = newVol;
  }
}
elem.volslider.onchange = volChanged;
let storedVol = localStorage.getItem("audioVolume");
if (storedVol) { elem.volslider.value = JSON.parse(storedVol); }
volChanged();

(function sendPos() {
  let qp = window.queued_pos;
  if (qp) {
    room.socket.send(`pos ${qp[0]} ${qp[1]}`);
    window.queued_pos = undefined;
  }
  setTimeout(function() {
    sendPos();
  }, 16);
})();
(function heartbeat() {
  setTimeout(function() {
    room.socket.send("<3");
    heartbeat();
  }, 30000);
})();
