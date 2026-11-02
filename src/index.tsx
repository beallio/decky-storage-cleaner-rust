import { callable, toaster } from "@decky/api";
import {
  ButtonItem,
  definePlugin,
  PanelSection,
  PanelSectionRow,
  staticClasses,
  DialogCheckbox,
  showModal,
  ConfirmModal
} from "@decky/ui";
import React, { FC, useState, useEffect, useRef } from "react";
import { FaBoxOpen } from "react-icons/fa";
import { getAppDetails } from "./utils";

type Kind = "shadercache" | "compatdata";

interface Entry {
  appid: string;
  bytes: number;
}

// Steam game (installed or not) or non-Steam shortcut.
type Group = "installed" | "uninstalled" | "shortcut";

interface Game extends Entry {
  name: string;
  group: Group;
  is_not_steam_cloud_supported: boolean;
}

interface DeleteResult {
  deleted: string[];
  failed: { appid: string; error: string }[];
}

const scan = callable<[kind: Kind], { entries: Entry[] }>("scan");
const deleteEntries = callable<[kind: Kind, appids: string[]], DeleteResult>("delete");

const SECTIONS: Record<Kind, { title: string; short: string; description: string }> = {
  shadercache: {
    title: "Shader Cache",
    short: "Shader Cache",
    description: "Shader cache is a precompiled collection of shader programs that helps reduce lag in graphics-intensive applications. It's ok to delete because it will be recreated the next time you run the application."
  },
  compatdata: {
    title: "Compatibility Data",
    short: "Compat Data",
    description: "Compatibility data is information stored by your Steam Deck to ensure compatibility with hardware and other software. It's ok to delete because it will be recreated automatically as needed."
  }
};

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${Math.round((bytes / 1024 ** i) * 100) / 100} ${units[i]}`;
}

// The confirm dialog closes the Quick Access menu, so the panel is mounted again while a delete
// may still run. A new panel waits for that delete before it scans, or it would list deleted folders.
const pendingDelete: Partial<Record<Kind, Promise<unknown>>> = {};

// Only entries that Steam can name are shown, so only those can be cleared.
async function loadGames(kind: Kind): Promise<Game[]> {
  await pendingDelete[kind]?.catch(() => undefined);
  const { entries } = await scan(kind);
  const details = await Promise.all(entries.map(({ appid }) => getAppDetails(parseInt(appid))));
  return entries.flatMap((entry, i) => {
    const info = details[i];
    if (!info?.strDisplayName || !entry.bytes) return [];
    // iInstallFolder is -1 both for shortcuts and for Steam games that are not installed.
    const group: Group = window.appStore.GetAppOverviewByAppID(parseInt(entry.appid))?.BIsShortcut()
      ? "shortcut"
      : info.iInstallFolder === -1 ? "uninstalled" : "installed";
    return [{
      ...entry,
      name: info.strDisplayName,
      group,
      is_not_steam_cloud_supported: info.eCloudStatus === 1
    }];
  });
}

const CacheSection: FC<{ kind: Kind }> = ({ kind }) => {
  const { title, short, description } = SECTIONS[kind];
  const isCompat = kind === "compatdata";
  const [games, setGames] = useState<Game[] | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);

  const refresh = () =>
    loadGames(kind)
      .then(setGames)
      .catch(e => {
        console.error(e);
        setGames([]);
      });

  useEffect(() => {
    refresh();
    return () => {
      mounted.current = false;
    };
  }, []);

  async function clear(appids: string[]) {
    setBusy(true);
    const job = deleteEntries(kind, appids);
    pendingDelete[kind] = job;
    try {
      const { deleted, failed } = await job;
      if (failed.length === 0) {
        // Deleting folders does not change the size of the others, so no new scan is needed.
        setGames(list => list && list.filter(({ appid }) => !deleted.includes(appid)));
        setSelected(ids => ids.filter(id => !deleted.includes(id)));
        return;
      }
      toaster.toast({
        title: `Could not clear ${failed.length} item(s)`,
        body: failed.map(({ appid, error }) => `${appid}: ${error}`).join("\n")
      });
    } catch (e) {
      toaster.toast({ title: "Storage Cleaner", body: String(e) });
    } finally {
      setBusy(false);
    }
    // A failed delete can leave part of a folder, so measure again; a panel that was
    // closed meanwhile is not shown, and its replacement scans after the delete.
    if (!mounted.current) return;
    setGames(null);
    setSelected([]);
    await refresh();
  }

  // Without a label, the dialog names every game in the list.
  function confirm(list: Game[], label?: string) {
    const unsafe = isCompat ? list.filter(({ is_not_steam_cloud_supported }) => is_not_steam_cloud_supported) : [];
    showModal(
      <ConfirmModal strTitle={`Clear ${title}`} strOKButtonText="Clear" onOK={() => clear(list.map(({ appid }) => appid))}>
        {unsafe.length > 0 && (
          <div style={{ color: "red", marginBottom: "10px" }}>
            <strong>DANGER: On-device game save data may be permanently lost for {unsafe.map(({ name }) => name).join(", ")}!</strong>
          </div>
        )}
        <div>
          Are you sure you want to clear the {title.toLowerCase()} for{" "}
          <strong>{label ?? list.map(({ name }) => name).join(", ")}</strong>?
        </div>
      </ConfirmModal>
    );
  }

  const group = (which: Group, label: string) => {
    const list = games?.filter(game => game.group === which) ?? [];
    if (list.length === 0) return null;
    return (
      <>
        <PanelSectionRow>
          <div style={{ fontSize: "15px", fontWeight: "bold", marginBottom: "10px" }}>{label}</div>
        </PanelSectionRow>
        {list.map(game => (
          <React.Fragment key={game.appid}>
            <DialogCheckbox
              label={`${game.name} (${formatBytes(game.bytes)})`}
              onChange={checked => setSelected(ids => (checked ? [...ids, game.appid] : ids.filter(id => id !== game.appid)))}
            />
            {isCompat && game.is_not_steam_cloud_supported && (
              <div style={{ fontSize: "12px", color: "red", margin: "0 0 12px 35px" }}>
                WARNING: This game DOES NOT support or has never synced to Steam Cloud Saves. ON-DEVICE GAME SAVE DATA MAY BE PERMANENTLY LOST!
              </div>
            )}
          </React.Fragment>
        ))}
      </>
    );
  };

  const chosen = games?.filter(({ appid }) => selected.includes(appid)) ?? [];
  const uninstalled = games?.filter(game => game.group === "uninstalled") ?? [];
  const total = games?.reduce((sum, { bytes }) => sum + bytes, 0) ?? 0;

  return (
    <PanelSection title={title} spinner={games === null || busy}>
      <PanelSectionRow>
        <div style={{ fontSize: "12px", marginBottom: "10px" }}>{description}</div>
      </PanelSectionRow>
      <PanelSectionRow>
        <div style={{ fontSize: "17px", marginBottom: "10px" }}>
          Total Size: {games ? formatBytes(total) : "Calculating..."}
        </div>
      </PanelSectionRow>
      {group("installed", "STEAM")}
      {group("uninstalled", "STEAM (NOT INSTALLED)")}
      {group("shortcut", "NON-STEAM")}
      <PanelSectionRow>
        <ButtonItem layout="below" bottomSeparator="none" disabled={busy || !games?.length} onClick={() => games && confirm(games, "ALL games")}>
          Clear All {short}
        </ButtonItem>
      </PanelSectionRow>
      <PanelSectionRow>
        <ButtonItem layout="below" bottomSeparator="none" disabled={busy || uninstalled.length === 0} onClick={() => confirm(uninstalled)}>
          Clear Not Installed {short}
        </ButtonItem>
      </PanelSectionRow>
      <PanelSectionRow>
        <ButtonItem layout="below" bottomSeparator="none" disabled={busy || chosen.length === 0} onClick={() => confirm(chosen)}>
          Clear Selected {short}
        </ButtonItem>
      </PanelSectionRow>
    </PanelSection>
  );
};

export default definePlugin(() => ({
  title: <div className={staticClasses.Title}>Storage Cleaner</div>,
  content: (
    <div id="decky-storage-cleaner">
      <CacheSection kind="shadercache" />
      <CacheSection kind="compatdata" />
    </div>
  ),
  icon: <FaBoxOpen />
}));
