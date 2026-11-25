# Inventory Explorer

Choose **Preferences → Aurora → Windows → Inventory → Explorer** to open your Second Life (SL) inventory with tabs, an editable path bar and a pinned sidebar. The setting also applies to an open inventory window. The other choices are **Classic window (Legacy)** for the single tree and **Aurora** for the adaptive inventory window.

A fresh installation starts with **Aurora**. Upgrades retain a saved inventory style. When reading older preferences, the viewer honors the old Classic switch first, then the Explorer switch.

## Find a folder or item

Click your avatar’s name to open **Home**. Open **General Inventory** for personal folders and loose items. Home also lists built-in categories, Library and system destinations, pinned folders and pin groups. **Other locations** keeps unpinned built-in destinations within reach. Home shows each section when it has entries.

- **Browse a path.** Click a parent in the path bar to go there. Press **Ctrl+L** to select the address, type or paste a path such as `\Appearance\Clothing`, then press **Enter**. If siblings share a name, choose the intended folder. A folder’s context menu offers **Copy folder path**. An item offers **Copy containing folder path**.
- **Search.** Press **Ctrl+F** or **F3** and enter part of an item or folder name. Searching from Home includes the entire inventory and Library. Searching inside a folder or category covers its displayed descendants. Results update as contents load. Right-click a hit and choose **Open containing folder** to select it at its displayed location, or **Show in original folder** for its real SL parent.
- **Retrace your steps.** **Alt+Left** goes back, including to a prior search and selection. **Alt+Right** goes forward and **Alt+Up** opens the parent.

## Arrange a view

Open a folder and choose **View → Details**, **List** or one of four icon sizes. Details has resizable columns. List flows down columns and scrolls horizontally. Hold **Ctrl** and scroll over the file pane to cycle through layouts. In icon views, eligible items can show a thumbnail. Right-click a folder or its pin and choose **Change folder icon…** for a local symbol, or **Restore default icon** to go back.

| Control | What it changes |
| --- | --- |
| **Sort** | Orders entries by **Name**, **Type** or **Date created**, ascending or descending. On Home, sorting applies within each section and the available keys are Name and Type. |
| **Sort → Group by** or **View → Group by** | Creates collapsible **Type**, **Name** or **Date created** sections in a folder or search. **None** restores a flat list. The direction in this submenu orders headings. The main sort setting orders items within each heading. |
| **Filter** | Shows matching inventory types or folders in a folder or search. Select **All** to clear it. In narrow windows, look under **…** for commands that no longer fit on the bar. |

Click a heading or its chevron to fold or expand that group. Its count reflects the current filter. Folding a group also removes its hidden rows from the selection. **Group by → Expand all groups** and **Collapse all groups** apply to the current view. Home keeps its own named, collapsible sections. Folders lack an SL creation timestamp and appear under **Unknown date** when grouped by date.

## Pin destinations and work across tabs

Right-click a folder and choose **Pin to sidebar**. In **View → Pinned folders**, you can check or uncheck several destinations before clicking outside the menu to dismiss it. **Open** navigates to a destination. Drag the sidebar’s right divider to change its width.

Drag sidebar pins to reorder them. To keep several together, right-click your avatar’s Home entry and choose **New pin group**. Drag a pin to the group header, or use the pin’s **Move to group** menu. Click the header to fold the group. Removing a group returns its pins to **Ungrouped**.

Open a new tab with **+** or **Ctrl+T**. Each tab shows its location’s icon and name and keeps its own location, selection and history during the session. Drag a tab past its neighbors to reorder it. Drag it out of a multi-tab strip to give it its own window. Drag a single-tab window by its tab and hold it over another tab strip to preview its position there. Release to dock, or press **Esc** to cancel. **Ctrl+Tab** and **Ctrl+Shift+Tab** cycle tabs. **Ctrl+W** closes the active tab. Closing a window’s last tab closes that window.

## Organize locally or change your SL inventory

**Dragging within Explorer changes where an entry appears on this computer.** For example, drop a pair of shoes onto a *Boots* folder and Explorer will show them there. To put the shoes into *Boots* for other viewers too, choose **Move in Second Life…** and select the real folder. If you later move the shoes in SL, an existing Explorer placement still controls their displayed location. **Restore original location** clears that placement.

| To | Action |
| --- | --- |
| Select entries | **Ctrl+click** toggles individual entries. **Shift+click** selects a displayed range. Drag from empty space for a selection rectangle, or press **Ctrl+A** in the file pane. |
| Organize a selection | Drag from a selected entry onto a writable Explorer destination, such as a personal folder or **Photos**. The drag label identifies the proposed destination. The icon or thumbnail follows the pointer, with a count for multiple entries. Hover another tab to switch to it. |
| Scroll while dragging | Move near or beyond the file pane’s top or bottom edge. Moving farther outside increases the speed. Use the left and right edges in **List** view. |
| Create a real folder | Choose **New → Folder** or press **Ctrl+Shift+N**, enter the name and press **Enter**. In **General Inventory**, the folder lives at your personal SL root. From **Home**, it lives under a real **Home Folders** container. In a combined category, choose the destination when prompted. |
| Edit real inventory | With the file pane focused, **F2** renames, **Delete** moves eligible entries to Trash, and **Ctrl+X**, **Ctrl+C**, **Ctrl+V** operate the inventory clipboard across tabs. The context menu also offers type-specific actions. **Paste as link** creates another SL reference to an eligible item. |

For worn clothing, removable attachments and active gestures, **Delete** asks for confirmation before taking them off or deactivating them. To permanently remove an entry already in Trash, right-click it, choose **Purge object** and confirm.

When an item is in the real **Trash**, its Restore command uses the original Second Life folder if the viewer remembers it and the folder still exists. Otherwise it uses the usual destination for that inventory type. **Restore original location** in a locally arranged view clears only the Explorer placement.

## Share items and use them in-world

Drag a real item, folder or eligible selection onto a resident in the scene, their profile, their private IM or their resident row in People, Contacts or the conversation list. Dropping a texture into a resident’s IM opens the standard **Share** dialog for that resident. Review the selection there and confirm to send it. SL transfer and no-copy permissions apply. For a group chat, choose an individual recipient to share with.

Other targets use their usual inventory actions. Drop a compatible object into the world to rez it where permitted. Object contents and compatible asset fields also accept inventory drags. A texture field, for example, accepts a single texture. An Explorer-only folder has a local placement and can be dragged within Explorer.

- **Environment presets.** Right-click a Sky, Water or Day Cycle and choose **Apply to myself only** for a personal preview. Select **Shared environment** in Aurora’s environment controls to return to the parcel or region setting. **Set as parcel environment…** targets the parcel under your avatar and asks you to confirm the parcel and preset. It requires permission to change that parcel’s environment.
- **Next-owner permissions.** Right-click eligible real items or folders and choose **Edit next-owner permissions…**. Choose **Keep current**, **Allow** or **Deny** for each permission. **Review selection** shows what would change or be skipped. Choose **Apply** and check the confirmed and unconfirmed counts. Selected folders include their real SL descendants. Inventory links and contents inside objects are excluded. Each item must retain Copy or Transfer.

## Back up your layout

Explorer saves pins, pin groups, local placements, folder icons, widths and folder-view preferences for this account on this computer. Close the viewer before copying `inventory-explorer-<inventory-root-uuid>.json` from Aurora’s account configuration directory. Earlier Explorer layouts load automatically and upgrade when edited. Saved paths using **My Folders** continue to open **General Inventory**.
