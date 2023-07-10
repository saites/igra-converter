<script setup lang="ts">
defineProps<{
    value?: Object | String,
}>()
</script>

<template>
  <td class="text-center"
    v-if="value !== undefined && value !== null">{{value}}</td>
  <td v-else class="text-center">-</td>
</template>
