# provision_users.cmake — Generates passwd/group files and home directories from users.conf
#
# Expected variables (passed via -D):
#   USERS_CONF  — path to users.conf
#   CONFIG_DIR  — path to config/ directory (for resolving template paths)
#   SYSROOT_DIR — path to build sysroot

cmake_minimum_required(VERSION 3.10)

if(NOT EXISTS "${USERS_CONF}")
  message(FATAL_ERROR "Users config not found: ${USERS_CONF}")
endif()

# Read config file
file(READ "${USERS_CONF}" CONF_CONTENT)

# Split into lines
string(REPLACE "\n" ";" CONF_LINES "${CONF_CONTENT}")

# State machine: parse [user] and [group] sections
set(SECTION "")
set(USER_COUNT 0)
set(GROUP_COUNT 0)

foreach(LINE ${CONF_LINES})
  # Skip empty lines and comments
  string(STRIP "${LINE}" LINE)
  if("${LINE}" STREQUAL "" OR "${LINE}" MATCHES "^#")
    continue()
  endif()

  # Section headers
  if("${LINE}" STREQUAL "[user]")
    # Save previous user if any
    if(SECTION STREQUAL "user" AND DEFINED CUR_NAME)
      # Store completed user
      set(USER_${USER_COUNT}_NAME "${CUR_NAME}")
      set(USER_${USER_COUNT}_PASSWORD "${CUR_PASSWORD}")
      set(USER_${USER_COUNT}_UID "${CUR_UID}")
      set(USER_${USER_COUNT}_GID "${CUR_GID}")
      set(USER_${USER_COUNT}_FULLNAME "${CUR_FULLNAME}")
      set(USER_${USER_COUNT}_HOMEDIR "${CUR_HOMEDIR}")
      set(USER_${USER_COUNT}_TEMPLATE "${CUR_TEMPLATE}")
      math(EXPR USER_COUNT "${USER_COUNT} + 1")
    endif()
    # Reset current fields
    set(SECTION "user")
    set(CUR_NAME "")
    set(CUR_PASSWORD "")
    set(CUR_UID "")
    set(CUR_GID "")
    set(CUR_FULLNAME "")
    set(CUR_HOMEDIR "")
    set(CUR_TEMPLATE "")
    continue()
  endif()

  if("${LINE}" STREQUAL "[group]")
    # Save previous user if any
    if(SECTION STREQUAL "user" AND DEFINED CUR_NAME)
      set(USER_${USER_COUNT}_NAME "${CUR_NAME}")
      set(USER_${USER_COUNT}_PASSWORD "${CUR_PASSWORD}")
      set(USER_${USER_COUNT}_UID "${CUR_UID}")
      set(USER_${USER_COUNT}_GID "${CUR_GID}")
      set(USER_${USER_COUNT}_FULLNAME "${CUR_FULLNAME}")
      set(USER_${USER_COUNT}_HOMEDIR "${CUR_HOMEDIR}")
      set(USER_${USER_COUNT}_TEMPLATE "${CUR_TEMPLATE}")
      math(EXPR USER_COUNT "${USER_COUNT} + 1")
    endif()
    # Save previous group if any
    if(SECTION STREQUAL "group" AND DEFINED CUR_GNAME)
      set(GROUP_${GROUP_COUNT}_NAME "${CUR_GNAME}")
      set(GROUP_${GROUP_COUNT}_GID "${CUR_GGID}")
      set(GROUP_${GROUP_COUNT}_MEMBERS "${CUR_GMEMBERS}")
      math(EXPR GROUP_COUNT "${GROUP_COUNT} + 1")
    endif()
    set(SECTION "group")
    set(CUR_GNAME "")
    set(CUR_GGID "")
    set(CUR_GMEMBERS "")
    continue()
  endif()

  # Parse key=value
  string(REGEX MATCH "^([^=]+)=(.*)" MATCH_RESULT "${LINE}")
  if(NOT MATCH_RESULT)
    continue()
  endif()
  set(KEY "${CMAKE_MATCH_1}")
  set(VAL "${CMAKE_MATCH_2}")

  if(SECTION STREQUAL "user")
    if(KEY STREQUAL "name")
      set(CUR_NAME "${VAL}")
    elseif(KEY STREQUAL "password")
      set(CUR_PASSWORD "${VAL}")
    elseif(KEY STREQUAL "uid")
      set(CUR_UID "${VAL}")
    elseif(KEY STREQUAL "gid")
      set(CUR_GID "${VAL}")
    elseif(KEY STREQUAL "fullname")
      set(CUR_FULLNAME "${VAL}")
    elseif(KEY STREQUAL "homedir")
      set(CUR_HOMEDIR "${VAL}")
    elseif(KEY STREQUAL "template")
      set(CUR_TEMPLATE "${VAL}")
    endif()
  elseif(SECTION STREQUAL "group")
    if(KEY STREQUAL "name")
      set(CUR_GNAME "${VAL}")
    elseif(KEY STREQUAL "gid")
      set(CUR_GGID "${VAL}")
    elseif(KEY STREQUAL "members")
      set(CUR_GMEMBERS "${VAL}")
    endif()
  endif()
endforeach()

# Save last section
if(SECTION STREQUAL "user" AND NOT "${CUR_NAME}" STREQUAL "")
  set(USER_${USER_COUNT}_NAME "${CUR_NAME}")
  set(USER_${USER_COUNT}_PASSWORD "${CUR_PASSWORD}")
  set(USER_${USER_COUNT}_UID "${CUR_UID}")
  set(USER_${USER_COUNT}_GID "${CUR_GID}")
  set(USER_${USER_COUNT}_FULLNAME "${CUR_FULLNAME}")
  set(USER_${USER_COUNT}_HOMEDIR "${CUR_HOMEDIR}")
  set(USER_${USER_COUNT}_TEMPLATE "${CUR_TEMPLATE}")
  math(EXPR USER_COUNT "${USER_COUNT} + 1")
elseif(SECTION STREQUAL "group" AND NOT "${CUR_GNAME}" STREQUAL "")
  set(GROUP_${GROUP_COUNT}_NAME "${CUR_GNAME}")
  set(GROUP_${GROUP_COUNT}_GID "${CUR_GGID}")
  set(GROUP_${GROUP_COUNT}_MEMBERS "${CUR_GMEMBERS}")
  math(EXPR GROUP_COUNT "${GROUP_COUNT} + 1")
endif()

# ── Generate passwd file ──
set(PASSWD_CONTENT "")
set(I 0)
while(I LESS USER_COUNT)
  set(UNAME "${USER_${I}_NAME}")
  set(UPASS "${USER_${I}_PASSWORD}")
  set(UUID "${USER_${I}_UID}")
  set(UGID "${USER_${I}_GID}")
  set(UFULL "${USER_${I}_FULLNAME}")
  set(UHOME "${USER_${I}_HOMEDIR}")

  # Default homedir
  if("${UHOME}" STREQUAL "")
    set(UHOME "/Users/${UNAME}")
  endif()

  # Hash password (empty = no password)
  set(UHASH "")
  if(NOT "${UPASS}" STREQUAL "")
    string(MD5 UHASH "${UPASS}")
  endif()

  # Format: username:hash:uid:gid:fullname:homedir
  string(APPEND PASSWD_CONTENT "${UNAME}:${UHASH}:${UUID}:${UGID}:${UFULL}:${UHOME}\n")

  math(EXPR I "${I} + 1")
endwhile()

# Write passwd
file(WRITE "${SYSROOT_DIR}/System/users/passwd" "${PASSWD_CONTENT}")
message(STATUS "Provisioned ${USER_COUNT} user(s) to passwd")

# ── Generate group file ──
set(GROUP_CONTENT "")
set(I 0)
while(I LESS GROUP_COUNT)
  set(GNAME "${GROUP_${I}_NAME}")
  set(GGID "${GROUP_${I}_GID}")
  set(GMEMBERS "${GROUP_${I}_MEMBERS}")

  # Format: groupname:gid:members
  string(APPEND GROUP_CONTENT "${GNAME}:${GGID}:${GMEMBERS}\n")

  math(EXPR I "${I} + 1")
endwhile()

# Write group
file(WRITE "${SYSROOT_DIR}/System/users/group" "${GROUP_CONTENT}")
message(STATUS "Provisioned ${GROUP_COUNT} group(s) to group")

# ── Create home directories and copy templates ──
set(I 0)
while(I LESS USER_COUNT)
  set(UNAME "${USER_${I}_NAME}")
  set(UHOME "${USER_${I}_HOMEDIR}")
  set(UTEMPLATE "${USER_${I}_TEMPLATE}")

  # Default homedir
  if("${UHOME}" STREQUAL "")
    set(UHOME "/Users/${UNAME}")
  endif()

  # Only create home dirs under /Users/ (skip root's "/" homedir)
  if("${UHOME}" MATCHES "^/Users/")
    set(SYSROOT_HOME "${SYSROOT_DIR}${UHOME}")

    # Copy template if specified
    if(NOT "${UTEMPLATE}" STREQUAL "")
      set(TEMPLATE_PATH "${CONFIG_DIR}/${UTEMPLATE}")
      if(EXISTS "${TEMPLATE_PATH}")
        file(COPY "${TEMPLATE_PATH}/" DESTINATION "${SYSROOT_HOME}")
        message(STATUS "Copied template '${UTEMPLATE}' to ${UHOME}")
      else()
        message(WARNING "Template directory not found: ${TEMPLATE_PATH}")
      endif()
    endif()

    # Ensure home dir and Documents exist
    file(MAKE_DIRECTORY "${SYSROOT_HOME}")
    file(MAKE_DIRECTORY "${SYSROOT_HOME}/Documents")
    message(STATUS "Created home directory: ${UHOME}")
  endif()

  math(EXPR I "${I} + 1")
endwhile()

message(STATUS "User provisioning complete")
