import { Host, Menu, Toggle } from '@expo/ui/swift-ui';
import { buttonStyle, labelStyle } from '@expo/ui/swift-ui/modifiers';

import { strings } from '@/chat/client/strings';
import { DEMO_USERS, findDemoUser, type DemoUserId } from '@/chat/shared/users';

/**
 * Header control that shows the current demo user and switches between Alice and Bob.
 */
export function UserMenu({
  userId,
  onSwitch,
}: {
  userId: DemoUserId;
  onSwitch: (userId: DemoUserId) => void;
}) {
  const current = findDemoUser(userId)?.displayName ?? userId;
  return (
    <Host matchContents style={{ minWidth: 44, minHeight: 44, justifyContent: 'center' }}>
      <Menu
        label={current}
        systemImage="person.crop.circle"
        modifiers={[buttonStyle('plain'), labelStyle('titleAndIcon')]}>
        {DEMO_USERS.map((user) => (
          <Toggle
            key={user.id}
            isOn={user.id === userId}
            label={user.displayName}
            onIsOnChange={(isOn) => {
              if (isOn) {
                onSwitch(user.id);
              }
            }}
          />
        ))}
      </Menu>
    </Host>
  );
}

/** Accessibility name for the header control. */
export function userMenuAccessibilityLabel(userId: DemoUserId): string {
  return `${strings.switchUser} ${findDemoUser(userId)?.displayName ?? userId}`;
}
