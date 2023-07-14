<script setup lang="ts">
import { friendlyProblem, friendlyFix } from '@/utils.js'
import { computed } from 'vue'

const props = defineProps({
  event: Object,
  issues: Array,
  partners: Array,
  relevant: Object,
})


const theseIssues = computed(() => {
  let { issues, event } = props;
  if (!issues || !event) { return []; }

  return issues.filter((issue) => {
      return (
        issue.problem.data && issue.problem.data.event === event.rodeoEventRelId
          && (
            !issue.problem.data.round
            || issue.problem.data.round === event.round
          )
      )
  })
})

const bgColor = computed(() => {
  return theseIssues.value.length ? "bg-red-400" : "#f9f9f9"
})

function problemName(issue) {
  return friendlyProblem(issue)
}

function suggestedFix(issue) {
  if (issue.fix.name === "UseThisRecord") {
    let other = props.relevant[issue.fix.data];
    if (other) {
      return `Maybe they meant ${other.first_name} ${other.last_name} with ID ${other.igra_number}?`
    }
  }
  return friendlyFix(issue)
}

const partnerInfo = computed(() => {
  let { event, partners, relevant } = props
  if (!event.partners || !partners) {
    return []
  }

  let eventPartners = partners.filter((p) => {
    return p.event === event.rodeoEventRelId && p.round === event.round
  });

  // TODO: this logic is buggy and misassociated some people.
  // Need to either move the logic over to the server,
  // or figure out what's broken here.
  // match the user value to the correct partner.
  return event.partners.map((nameOrNum) => {
    let lowerNN = nameOrNum.trim().toLowerCase()
    if (lowerNN === "") { return {} }

    // among their known partners, find a match
    // if the lowerNN contains the IGRA number or partner name
    return eventPartners.find((p) => {
      if (lowerNN.includes(p.igra_number)) { return true }

      let dbPartner = relevant[p.igra_number]
      if (!dbPartner) { return false }

      let partnerName = `${dbPartner.first_name} ${dbPartner.last_name}`.toLowerCase()
      let partnerLegalName = `${dbPartner.legal_first} ${dbPartner.legal_last}`.toLowerCase()
      return lowerNN.includes(partnerName) || lowerNN.includes(partnerLegalName)
    })
  }).map((p) => { return p ? relevant[p.igra_number] : undefined })
})

function formatPartner(p) {
  if (!p) { return }
  return `${p.igra_number} ${p.first_name} ${p.last_name}`
}
</script>

<template>
  <tr :class="bgColor" class="text-center">
    <td>{{event.rodeoEventRelId}}</td>
    <td>{{event.round}}</td>
    <td>{{event.partners[0]}}</td>
    <td>{{event.partners[1]}}</td>
  </tr>
  <tr :class="bgColor" class="text-left text-sm" v-for="issue in theseIssues">
    <td colspan="4">
      → <span>{{problemName(issue)}}</span> <span>{{suggestedFix(issue)}}</span>
    </td>
  </tr>
  <tr>
    <td></td>
    <td></td>
    <td v-if="partnerInfo[0]">Match: {{formatPartner(partnerInfo[0])}}</td>
    <td v-else></td>
    <td v-if="partnerInfo[1]">Match: {{formatPartner(partnerInfo[1])}}</td>
    <td v-else></td>
  </tr>
  <tr>
    <td></td>
    <td></td>
    <td></td>
    <td></td>
  </tr>
</template>

